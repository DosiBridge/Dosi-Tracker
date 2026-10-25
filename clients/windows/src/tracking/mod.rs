//! Tracking subsystem.
//!
//! Design goal: keep idle CPU near zero. Input listening is event-driven
//! (the OS pushes events; we only record timestamps), the foreground window
//! is sampled every few seconds with a couple of Win32 calls, and heavy work
//! (screenshot, webcam) happens once per block, then the agent sleeps.

pub mod active_window;
pub mod input;
pub mod lifecycle;
pub mod screenshot;
pub mod system;
pub mod usage;
pub mod webcam;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::CapturePermissions;
use crate::model::{Activity, EndReason, WindowInfo};
use input::InputCounter;
pub use lifecycle::{FakeSensorDriver, NativeSensorDriver, SensorDriver};
use screenshot::ScreenCapture;
use system::DeviceInfo;
use usage::{UsageRecorder, UsageSlice};

#[derive(Debug, Clone)]
pub struct ProvenanceScreenCapture {
    pub capture: ScreenCapture,
    pub generation: u64,
}


/// A block shorter than this is never recorded (CONTRACT §1).
pub const MIN_BLOCK: TimeDelta = TimeDelta::seconds(60);

/// The longest task note the server stores.
const MAX_DESCRIPTION_CHARS: usize = 2048;

/// One block to record: its project and capture policy, its time window, and
/// why it ended.
#[derive(Debug, Clone, Copy)]
pub struct BlockSpec<'a> {
    pub project_id: &'a str,
    pub perms: &'a CapturePermissions,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub end_reason: Option<EndReason>,
    /// The task note ("What are you working on?").
    pub description: Option<&'a str>,
}

impl BlockSpec<'_> {
    /// May the block take a capture now that it ends? Not when the machine
    /// is locked, asleep, idle or shutting down.
    pub fn captures_at_end(&self) -> bool {
        self.end_reason.is_none_or(EndReason::allows_capture_at_end)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityGenerations {
    pub screen: u64,
    pub webcam: u64,
    pub window: u64,
    pub processes: u64,
}

impl Default for CapabilityGenerations {
    fn default() -> Self {
        Self {
            screen: 1,
            webcam: 1,
            window: 1,
            processes: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct TrackerAuth {
    active: bool,
    perms: CapturePermissions,
    generations: CapabilityGenerations,
}

impl Default for TrackerAuth {
    fn default() -> Self {
        Self {
            active: false,
            perms: CapturePermissions::all_disabled(),
            generations: CapabilityGenerations::default(),
        }
    }
}

impl TrackerAuth {
    /// Update effective authorization and advance capability generations if and only if
    /// an authorized capability was revoked, paused/stopped, or tightened (e.g. blur mandated),
    /// or if a new authorization era begins (granting from unauthorized).
    ///
    /// Preserves existing generations for unchanged or unaffected capabilities
    /// so routine repeated publication of identical policy does not invalidate
    /// legitimate in-flight sampling.
    fn update(&mut self, active: bool, perms: Option<&CapturePermissions>) {
        let new_active = active && perms.is_some();
        let new_perms = if active {
            perms.cloned().unwrap_or_else(CapturePermissions::all_disabled)
        } else {
            CapturePermissions::all_disabled()
        };

        // 1. Screen generation: transitions in authorization or blur tightening
        let screen_was_auth = self.active && self.perms.screenshot;
        let screen_is_auth = new_active && new_perms.screenshot;
        let blur_tightened = screen_was_auth && screen_is_auth && !self.perms.blur_screenshots && new_perms.blur_screenshots;
        if screen_was_auth != screen_is_auth || blur_tightened {
            self.generations.screen = self.generations.screen.wrapping_add(1);
        }

        // 2. Webcam generation: transitions in authorization
        let webcam_was_auth = self.active && self.perms.webcam;
        let webcam_is_auth = new_active && new_perms.webcam;
        if webcam_was_auth != webcam_is_auth {
            self.generations.webcam = self.generations.webcam.wrapping_add(1);
        }

        // 3. Window generation: transitions in authorization
        let window_was_auth = self.active && self.perms.active_window;
        let window_is_auth = new_active && new_perms.active_window;
        if window_was_auth != window_is_auth {
            self.generations.window = self.generations.window.wrapping_add(1);
        }

        // 4. Processes generation: transitions in authorization
        let progs_was_auth = self.active && self.perms.running_programs;
        let progs_is_auth = new_active && new_perms.running_programs;
        if progs_was_auth != progs_is_auth {
            self.generations.processes = self.generations.processes.wrapping_add(1);
        }

        self.active = new_active;
        self.perms = new_perms;
    }
}

/// Collects one activity snapshot for the given project time block.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
pub struct Tracker {
    input: Arc<InputCounter>,
    usage: Arc<UsageRecorder>,
    device: DeviceInfo,
    driver: Arc<dyn SensorDriver>,
    acquisition: Arc<dyn AcquisitionProvider>,
    auth: RwLock<TrackerAuth>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::with_driver_device_and_acquisition(
            Arc::new(NativeSensorDriver::new()),
            DeviceInfo::collect(),
            Arc::new(NativeAcquisitionProvider::new()),
        )
    }

    /// Construct a Tracker with an explicit sensor driver.
    ///
    /// Invariant: Constructor NEVER starts input hooks or window sampler!
    /// Sensors remain completely stopped and unattached until explicit authorized activation.
    #[allow(dead_code)]
    pub fn with_driver_and_device(driver: Arc<dyn SensorDriver>, device: DeviceInfo) -> Self {
        Self::with_driver_device_and_acquisition(
            driver,
            device,
            Arc::new(NativeAcquisitionProvider::new()),
        )
    }

    #[allow(dead_code)]
    pub fn with_driver(driver: Arc<dyn SensorDriver>) -> Self {
        Self::with_driver_and_device(driver, DeviceInfo::collect())
    }

    pub fn with_driver_device_and_acquisition(
        driver: Arc<dyn SensorDriver>,
        device: DeviceInfo,
        acquisition: Arc<dyn AcquisitionProvider>,
    ) -> Self {
        let input = Arc::new(InputCounter::new());
        let usage = Arc::new(UsageRecorder::new());
        Self {
            input,
            usage,
            device,
            driver,
            acquisition,
            auth: RwLock::new(TrackerAuth::default()),
        }
    }

    /// Construct a fake tracker for safe synthetic testing without OS hooks and without disk side effects.
    #[cfg_attr(not(test), expect(dead_code, reason = "for testing and inspection"))]
    pub fn new_fake() -> (Self, Arc<FakeSensorDriver>) {
        let (tracker, driver, _acq) = Self::new_fake_with_acquisition();
        (tracker, driver)
    }

    /// Construct a fake tracker exposing both fake driver and fake acquisition provider.
    #[cfg_attr(not(test), expect(dead_code, reason = "for testing and inspection"))]
    pub fn new_fake_with_acquisition() -> (Self, Arc<FakeSensorDriver>, Arc<FakeAcquisitionProvider>) {
        let driver = Arc::new(FakeSensorDriver::new());
        let acquisition = Arc::new(FakeAcquisitionProvider::new());
        (
            Self::with_driver_device_and_acquisition(driver.clone(), DeviceInfo::fixture(), acquisition.clone()),
            driver,
            acquisition,
        )
    }

    /// Read-only access to input for the live view (counts and activity % of
    /// the block in progress, whether input capture works).
    pub fn input(&self) -> Arc<InputCounter> {
        self.input.clone()
    }

    /// Read-only access to foreground-window usage, for the window's "top
    /// apps" list (shown only when the project records app usage).
    pub fn usage(&self) -> Arc<UsageRecorder> {
        self.usage.clone()
    }

    /// Access the underlying sensor driver.
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn driver(&self) -> Arc<dyn SensorDriver> {
        self.driver.clone()
    }

    /// Access the underlying acquisition provider.
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn acquisition(&self) -> Arc<dyn AcquisitionProvider> {
        self.acquisition.clone()
    }

    /// Apply effective sensor policy based on active tracking status and session permissions.
    ///
    /// When inactive or perms are None, detaches OS hooks and halts sampling immediately,
    /// failing closed to unauthorized for all capabilities.
    /// When active, attaches only the sensors explicitly permitted by policy and retains
    /// current effective authorization at the acquisition boundary.
    ///
    /// Synchronization Contract:
    /// - Atomically updates `auth` under a write lock, advancing per-capability invalidation
    ///   generations on authorization transitions (revocation, grant, or tightening).
    /// - Routine repeated publication of an IDENTICAL policy preserves generations, avoiding
    ///   false invalidation of legitimate in-flight acquisitions.
    /// - Because acquisition operations NEVER hold a read lock across provider calls,
    ///   `apply_sensor_policy` does not suffer authorization lock contention.
    /// - IMPORTANT: `apply_sensor_policy` does NOT complete instantaneously; it synchronously
    ///   posts quit messages and joins native sensor driver threads (`driver.stop_input`, `driver.stop_usage`).
    ///   Absence of auth-lock contention is narrower than total nonblocking completion.
    pub fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>) {
        let active_window = active && perms.is_some_and(|p| p.active_window);
        let keyboard = active && perms.is_some_and(|p| p.keyboard);
        let mouse = active && perms.is_some_and(|p| p.mouse);
        let input_active = keyboard || mouse;

        // 1. Monotonically update internal acquisition authorization under lock.
        // Fails closed to inactive and all-disabled when active is false or perms are None.
        // Advances capability generations when authorized capabilities are revoked, granted,
        // or tightened, preventing ABA revocation/regrant resurrection of in-flight results.
        {
            let mut auth = self.auth.write().unwrap();
            auth.update(active, perms);
        }

        // 2. Update usage & input accumulators.
        self.usage.set_active(active_window);
        self.input.set_policy(input_active, keyboard, mouse);

        // 3. Update sensor drivers.
        if input_active {
            self.driver.start_input(self.input.clone(), keyboard, mouse);
        } else {
            self.driver.stop_input();
        }

        if active_window {
            self.driver.start_usage(self.usage.clone());
        } else {
            self.driver.stop_usage();
        }
    }

    /// Turn active sensor collection on or off (legacy helper using default permissions).
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn set_tracking_active(&self, active: bool) {
        if active {
            let default_perms = CapturePermissions::default();
            self.apply_sensor_policy(true, Some(&default_perms));
        } else {
            self.apply_sensor_policy(false, None);
        }
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "for UI and inspection"))]
    pub fn is_tracking_active(&self) -> bool {
        self.auth.read().unwrap().active
    }

    /// The app in front now (its executable name), queried through the acquisition provider
    /// ONLY IF actively tracking and active_window is authorized in current policy.
    ///
    /// Admission / Revocation Synchronization Contract:
    /// - Linearized logical admission: evaluates authorization and captures capability admission
    ///   generation token under a brief read lock.
    /// - Drops read lock BEFORE delegating to provider so sensor driver state changes do not contend
    ///   for authorization locks across slow foreground queries.
    /// - A brief read-lock check followed by an unlocked provider invocation linearizes logical admission,
    ///   not atomic physical provider entry with revocation.
    /// - Post-admission validation: validates generation token upon return. If active_window was revoked
    ///   or policy tightened while in flight (even if later re-granted before return), generation token
    ///   mismatch proves the sample originated in an invalidated consent interval, and the result is discarded.
    /// - Note: Discarding the result prevents admission into storage or state; it does not cancel in-flight OS work.
    pub fn current_app(&self) -> Option<String> {
        let (authorized, token) = {
            let auth = self.auth.read().unwrap();
            (auth.active && auth.perms.active_window, auth.generations.window)
        };
        if !authorized {
            return None;
        }

        let win = self.acquisition.current_window()?;

        // Post-admission verification: validate authorization and generation token.
        let still_authorized = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.active_window && auth.generations.window == token
        };
        if !still_authorized {
            return None;
        }

        let name = win.app_name;
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// This device: install id, name, OS and agent version (also for the
    /// presence heartbeat).
    #[expect(dead_code, reason = "for the worker's presence heartbeat")]
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// Capture the screen now. The caller picks the moment — a random point
    /// inside the block — and hands the capture to [`Tracker::snapshot`] when
    /// the block ends.
    ///
    /// Admission / Revocation Synchronization Contract:
    /// 1. Linearized logical admission: gated on CURRENT effective authorization (`auth.active && auth.perms.screenshot`).
    /// 2. If unauthorized, returns `None` immediately with 0 provider calls.
    /// 3. Read lock is dropped before entering the provider so `apply_sensor_policy`
    ///    does not suffer authorization lock contention.
    /// 4. Blur requirement is evaluated conservatively: blurred if either the caller
    ///    requested blur OR current effective policy requires blur.
    /// 5. Generation token captured at admission: on provider return, verifies that
    ///    `auth.generations.screen == token`. If screenshot was revoked or blur requirement
    ///    was added while in flight (even if later re-granted before return), generation
    ///    token mismatch ensures the invalidated capture is discarded.
    ///
    /// Note on physical execution vs logical admission:
    /// A short read-lock check followed by an unlocked provider invocation linearizes logical admission,
    /// but does not guarantee atomic physical provider entry with revocation. Synchronous OS screenshot
    /// capture cannot be cancelled mid-flight without unsafe aborts. Discarding the result prevents
    /// invalidated captures from entering storage or activity blocks; it does not constitute hardware
    /// cancellation or sensor stopping. Holding a global auth lock across blocking OS work is deliberately
    /// avoided to prevent deadlocks and UI thread lockups.
    pub fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        let (authorized, effective_blur, token) = {
            let auth = self.auth.read().unwrap();
            (
                auth.active && auth.perms.screenshot,
                blur || auth.perms.blur_screenshots,
                auth.generations.screen,
            )
        };

        if !authorized {
            return None;
        }

        let capture = match self.acquisition.capture_screen(effective_blur) {
            Ok(capture) => capture,
            Err(e) => {
                tracing::warn!(?e, "screenshot capture failed");
                return None;
            }
        };

        // Post-admission verification: check if revoked, regranted, or tightened while in-flight
        let still_valid = {
            let auth = self.auth.read().unwrap();
            auth.active
                && auth.perms.screenshot
                && auth.generations.screen == token
                && (!auth.perms.blur_screenshots || capture.blurred)
        };

        if !still_valid {
            tracing::debug!("screenshot completed after revocation or policy change; discarding capture");
            return None;
        }

        Some(ProvenanceScreenCapture {
            capture,
            generation: token,
        })
    }


    /// Build the activity for `block`, with `screen` taken earlier by
    /// [`Tracker::capture_screen`] (if any).
    ///
    /// Admission / Revocation Synchronization Contract:
    /// - Upfront Generation Era Token Capture: Generation era tokens are captured upfront
    ///   under a brief read lock for all capabilities (`window_token`, `progs_token`, `screen_token`,
    ///   `webcam_token`) at method entry.
    /// - Sequential Provider Execution with Fresh Boolean Checks: Immediately before calling each
    ///   provider (`current_window`, `running_programs`, `capture_webcam`), a fresh boolean admission
    ///   check is evaluated under a brief read lock (`auth.active && auth.perms.<cap> && perms.<cap>`).
    ///   If unauthorized, the provider call is skipped entirely (zero provider calls).
    /// - Read locks are NEVER held across provider calls, avoiding authorization lock contention
    ///   with `apply_sensor_policy`.
    /// - Conservative Rejection after Generation Changes: If a capability undergoes an authorization
    ///   transition (revocation, re-grant, or blur tightening) after snapshot start, a later provider
    ///   may execute under current boolean permissions if re-granted, but the resulting acquisition
    ///   will be conservatively rejected at final publication reconciliation because the current
    ///   generation token will no longer match the upfront era token.
    /// - Final Post-Provider Reconciliation: Once all sequential provider calls finish, a single
    ///   post-execution validation under lock validates all acquired results against current authorization,
    ///   the upfront era tokens, and (for screen) the handed-off screen provenance generation token.
    ///   - Discarding invalidated results prevents unconsented data from entering activity blocks
    ///     or persistent storage; it is post-execution validation, not hardware cancellation or
    ///     atomic physical provider entry with revocation.
    ///   - `self.usage.record(ended_at, ...)` is deferred to this final reconciliation and invoked
    ///     if and only if `window_valid` passes, guaranteeing that invalidated window samples
    ///     never contaminate historical usage.
    /// - Process Decoupling: `running_programs` is queried based strictly on active running_programs
    ///   authorization and historical block permission, independent of the active_window usage driver.
    /// - Historical Accumulator Preservation: Historical accumulator data (`slices`, `key_presses`,
    ///   `clicks`, `active_seconds`) recorded while consent was active is preserved in the emitted block
    ///   without acquiring new data.
    /// - Screenshot Provenance Handoff: Handed-off screenshots must have been captured under an authorized
    ///   generation matching the current publication era, and conservative blur is enforced if either
    ///   the caller or current policy requires blur.
    pub fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<ProvenanceScreenCapture>) -> Activity {
        let perms = block.perms;
        let (started_at, ended_at) = (block.started_at, block.ended_at);

        // 0. Capture generation tokens for all capabilities upfront before invoking any providers
        let (window_token, progs_token, screen_token, webcam_token) = {
            let auth = self.auth.read().unwrap();
            (
                auth.generations.window,
                auth.generations.processes,
                auth.generations.screen,
                auth.generations.webcam,
            )
        };

        // 1. Current window admission check: requires actively authorized in current policy,
        // allowed in historical block, and usage sampler is running.
        let admit_window = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.active_window && perms.active_window && self.driver.is_usage_running()
        };
        let window_raw = if admit_window {
            self.acquisition.current_window()
        } else {
            None
        };

        // 2. Running programs admission check: requires actively authorized in current policy
        // and allowed in historical block. Strictly independent of the active_window usage driver.
        let admit_programs = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.running_programs && perms.running_programs
        };
        let running_programs_raw = if admit_programs {
            match self.acquisition.running_programs() {
                Ok(progs) => progs,
                Err(e) => {
                    tracing::warn!(?e, "could not list running programs");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // 3. Screen capture provenance preparation: verify screen passed in from caller against historical block permission.
        let screen_raw = if perms.screenshot {
            screen
        } else {
            None
        };

        // 4. Webcam capture admission check: requires actively authorized in current policy,
        // allowed in historical block, and block allows capture at end.
        let admit_webcam = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.webcam && perms.webcam && block.captures_at_end()
        };
        let webcam_jpg_base64_raw = if admit_webcam {
            match self.acquisition.capture_webcam() {
                Ok(b64) => Some(b64),
                Err(e) => {
                    tracing::warn!(?e, "webcam capture failed");
                    None
                }
            }
        } else {
            None
        };

        // 5. Final Logical Publication Boundary & Post-All-Providers Revalidation:
        // Verify that NONE of the capability generations were revoked, re-granted (ABA), or altered
        // during the execution of any earlier or later provider calls.
        let (window_now, running_programs, screen, webcam_jpg_base64) = {
            let auth = self.auth.read().unwrap();
            let window_valid = auth.active && auth.perms.active_window && auth.generations.window == window_token;
            let progs_valid = auth.active && auth.perms.running_programs && auth.generations.processes == progs_token;
            let blur_required = auth.perms.blur_screenshots || perms.blur_screenshots;
            let screen_valid = auth.active
                && auth.perms.screenshot
                && auth.generations.screen == screen_token
                && screen_raw.as_ref().is_some_and(|sc| sc.generation == screen_token)
                && screen_raw.as_ref().is_some_and(|sc| sc.capture.blurred || !blur_required);
            let webcam_valid = auth.active && auth.perms.webcam && auth.generations.webcam == webcam_token;

            let final_window = if window_valid {
                if let Some(ref win) = window_raw {
                    self.usage.record(ended_at.timestamp_millis(), Some(win.clone()));
                }
                window_raw
            } else {
                None
            };

            let final_progs = if progs_valid {
                running_programs_raw
            } else {
                Vec::new()
            };

            let final_screen = if screen_valid {
                screen_raw.map(|sc| sc.capture)
            } else {
                None
            };

            let final_webcam = if webcam_valid {
                webcam_jpg_base64_raw
            } else {
                None
            };

            (final_window, final_progs, final_screen, final_webcam)
        };



        let measured = Measurements {
            key_presses: if perms.keyboard { self.input.key_presses_between(started_at, ended_at) } else { Vec::new() },
            clicks: if perms.mouse { self.input.clicks_between(started_at, ended_at) } else { Vec::new() },
            active_seconds: self.input.active_seconds(started_at, ended_at, perms.keyboard, perms.mouse),
            injected: self.input.injected_between(started_at, ended_at, perms.keyboard, perms.mouse),
            input_available: self.input.capture_available_between(started_at, ended_at),
            slices: if perms.active_window {
                self.usage.slices_between(started_at.timestamp_millis(), ended_at.timestamp_millis())
            } else {
                Vec::new()
            },
            window_now,
            running_programs,
            screen,
            webcam_jpg_base64,
        };
        let zone = Zone { time_zone: system::time_zone(), utc_offset_minutes: system::utc_offset_minutes(started_at) };
        build_activity(block, measured, &self.device, zone)
    }

    /// The away span kept as work after the idle prompt; see [`kept_idle_blocks`].
    #[expect(dead_code, reason = "called by the worker's idle prompt")]
    pub fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: std::time::Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        let max = TimeDelta::from_std(interval).unwrap_or(TimeDelta::minutes(10));
        kept_idle_blocks(project_id, from, to, max, description, &self.device, system::time_zone().as_deref())
    }
}

/// What the tracker measured for a block — read from the OS by
/// [`Tracker::snapshot`], turned into the payload by [`build_activity`].
pub struct Measurements {
    /// Unix ms of key presses (empty when the keyboard is not recorded).
    pub key_presses: Vec<i64>,
    /// Unix ms of clicks (empty when the mouse is not recorded).
    pub clicks: Vec<i64>,
    /// Unix seconds with input of the recorded kinds.
    pub active_seconds: Vec<i64>,
    /// Injected events of the recorded kinds.
    pub injected: u64,
    /// The input hooks worked for the whole block.
    pub input_available: bool,
    /// Foreground usage (empty when the active window is not recorded).
    pub slices: Vec<UsageSlice>,
    /// The window in front at the end of the block.
    pub window_now: Option<WindowInfo>,
    pub running_programs: Vec<WindowInfo>,
    pub screen: Option<ScreenCapture>,
    pub webcam_jpg_base64: Option<String>,
}

/// The device's time zone when the block started.
pub struct Zone {
    pub time_zone: Option<String>,
    pub utc_offset_minutes: i32,
}

/// Assemble the upload payload of a block (CONTRACT §1).
///
/// Activity is "not measured" — `productivity`, `idleSeconds` and
/// `injectedInputEvents` left out, and no per-minute timeline (whose
/// `active: false` minutes would read as idle) — when the project records
/// neither keyboard nor mouse, or the input hooks were not working for the
/// whole block.
pub fn build_activity(block: &BlockSpec<'_>, measured: Measurements, device: &DeviceInfo, zone: Zone) -> Activity {
    let perms = block.perms;
    let (start, end) = (block.started_at.timestamp(), block.ended_at.timestamp());
    let input_measured = (perms.keyboard || perms.mouse) && measured.input_available;

    let active_windows: Vec<WindowInfo> = if !perms.active_window {
        Vec::new()
    } else if measured.slices.is_empty() {
        // Too short for a single sample: fall back to the window in front.
        measured.window_now.into_iter().collect()
    } else {
        usage::app_usage(&measured.slices, &measured.key_presses, &measured.clicks)
    };
    let timeline = if input_measured {
        usage::minute_timeline(
            start,
            end,
            &measured.slices,
            &measured.key_presses,
            &measured.clicks,
            &measured.active_seconds,
        )
    } else {
        Vec::new()
    };

    // Only a capture the project allows, and blurred when it must be: the
    // server would drop an unblurred one anyway.
    let screen = measured
        .screen
        .filter(|_| perms.screenshot)
        .filter(|capture| capture.blurred || !perms.blur_screenshots);
    let screenshot_captured_at = screen.as_ref().map(|capture| capture.captured_at);
    let screenshot_blurred = screen.as_ref().map(|capture| capture.blurred);
    let (screenshot, thumb) = match screen {
        Some(capture) => (Some(capture.jpeg_base64), capture.thumb_jpg_base64),
        None => (None, None),
    };

    let activity = Activity {
        description: clean_description(block.description),
        productivity: input_measured.then(|| input::activity_percent(&measured.active_seconds, start, end)),
        mouse_clicks: measured.clicks.len() as u64,
        keyboard_hits: measured.key_presses.len() as u64,
        active_windows,
        running_programs: measured.running_programs,
        idle_seconds: input_measured.then(|| usage::idle_seconds(&timeline, start, end)),
        timeline,
        screenshot_png_base64: screenshot,
        screenshot_thumb_jpg_base64: thumb,
        webcam_jpg_base64: measured.webcam_jpg_base64.filter(|_| perms.webcam),
        end_reason: block.end_reason,
        screenshot_captured_at,
        screenshot_blurred,
        injected_input_events: input_measured.then(|| u32::try_from(measured.injected).unwrap_or(u32::MAX)),
        ..Activity::new(block.project_id, block.started_at, block.ended_at)
    };
    with_device_context(activity, device, zone)
}

/// Stamp the device context (CONTRACT §1) on a block.
fn with_device_context(activity: Activity, device: &DeviceInfo, zone: Zone) -> Activity {
    Activity {
        platform: Some(device.platform.to_string()),
        os_version: device.os_version.clone(),
        agent_version: Some(device.agent_version.to_string()),
        device_id: device.device_id.clone(),
        device_name: device.device_name.clone(),
        time_zone: zone.time_zone,
        utc_offset_minutes: Some(zone.utc_offset_minutes.clamp(-840, 840)),
        ..activity
    }
}

/// The task note as sent: trimmed, within the server's limit, `None` when blank.
fn clean_description(description: Option<&str>) -> Option<String> {
    let text: String = description?.trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// Split `from .. to` into back-to-back pieces of at most `max` each, as
/// equal as possible (no sliver at the end). Nothing when the span is shorter
/// than [`MIN_BLOCK`].
pub fn split_span(from: DateTime<Utc>, to: DateTime<Utc>, max: TimeDelta) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let total_ms = (to - from).num_milliseconds();
    let max_ms = max.num_milliseconds();
    if to - from < MIN_BLOCK || max_ms <= 0 {
        return Vec::new();
    }
    let pieces = (total_ms + max_ms - 1) / max_ms;
    let boundary = |i: i64| {
        if i == pieces {
            to
        } else {
            from + TimeDelta::milliseconds(total_ms * i / pieces)
        }
    };
    (0..pieces).map(|i| (boundary(i), boundary(i + 1))).collect()
}

/// The away span kept as work after the idle prompt (CONTRACT §4): blocks of
/// at most one interval, 0% active and idle for their whole length, ended by
/// `idle`, without windows, timeline or captures, carrying the task note.
pub fn kept_idle_blocks(
    project_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    interval: TimeDelta,
    description: Option<&str>,
    device: &DeviceInfo,
    time_zone: Option<&str>,
) -> Vec<Activity> {
    split_span(from, to, interval)
        .into_iter()
        .map(|(start, end)| {
            let activity = Activity {
                description: clean_description(description),
                productivity: Some(0),
                idle_seconds: Some(u32::try_from((end - start).num_seconds()).unwrap_or(u32::MAX)),
                end_reason: Some(EndReason::Idle),
                ..Activity::new(project_id, start, end)
            };
            let zone = Zone {
                time_zone: time_zone.map(str::to_string),
                utc_offset_minutes: system::utc_offset_minutes(start),
            };
            with_device_context(activity, device, zone)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).unwrap()
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            device_id: Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into()),
            device_name: Some("DESKTOP-1".into()),
            os_version: Some("Windows 11 Pro 23H2 (10.0.22631)".into()),
            platform: "windows",
            agent_version: "9.9.9",
        }
    }

    fn zone() -> Zone {
        Zone { time_zone: Some("Asia/Dhaka".into()), utc_offset_minutes: 360 }
    }

    fn all_allowed() -> CapturePermissions {
        CapturePermissions::default()
    }

    fn slice(app: &str, from_s: i64, to_s: i64) -> UsageSlice {
        UsageSlice { app: app.into(), title: format!("{app} window"), start_ms: T0_MS + from_s * 1000, end_ms: T0_MS + to_s * 1000 }
    }

    /// Ten minutes of measured work: typing in minutes 0-5, clicks in 7,
    /// nothing in 6, 8 and 9; three injected events.
    fn measurements() -> Measurements {
        let key_presses: Vec<i64> = (0..6).map(|m| T0_MS + m * 60_000 + 5_000).collect();
        let clicks = vec![T0_MS + 7 * 60_000 + 1_000];
        let mut active_seconds: Vec<i64> = key_presses.iter().map(|ms| ms / 1000).collect();
        active_seconds.extend(clicks.iter().map(|ms| ms / 1000));
        Measurements {
            key_presses,
            clicks,
            active_seconds,
            injected: 3,
            input_available: true,
            slices: vec![slice("code.exe", 0, 400), slice("chrome.exe", 400, 600)],
            window_now: None,
            running_programs: vec![WindowInfo { app_name: "slack.exe".into(), ..Default::default() }],
            screen: None,
            webcam_jpg_base64: None,
        }
    }

    fn capture(blurred: bool) -> ScreenCapture {
        ScreenCapture {
            jpeg_base64: "/9j/screen".into(),
            thumb_jpg_base64: Some("/9j/thumb".into()),
            captured_at: at(T0 + 250),
            blurred,
        }
    }

    fn prov_cap(tracker: &Tracker, blurred: bool) -> ProvenanceScreenCapture {
        let generation = tracker.auth.read().unwrap().generations.screen;
        ProvenanceScreenCapture {
            capture: capture(blurred),
            generation,
        }
    }

    fn block<'a>(perms: &'a CapturePermissions, end_reason: Option<EndReason>) -> BlockSpec<'a> {
        BlockSpec {
            project_id: "p1",
            perms,
            started_at: at(T0),
            ended_at: at(T0 + 600),
            end_reason,
            description: Some("  Invoice export  "),
        }
    }

    #[test]
    fn a_measured_block_carries_every_contract_field() {
        let perms = all_allowed();
        let activity = build_activity(&block(&perms, Some(EndReason::Interval)), measurements(), &device(), zone());

        // Active in minutes 0-5 and 7: 7 of 10 buckets.
        assert_eq!(activity.productivity, Some(70));
        assert_eq!(activity.idle_seconds, Some(3 * 60));
        assert_eq!(activity.injected_input_events, Some(3));
        assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (6, 1));
        assert_eq!(activity.timeline.len(), 10);
        assert_eq!(activity.timeline[0].active_seconds, Some(1));
        assert_eq!(activity.timeline[6].active_seconds, Some(0));
        assert!(!activity.timeline[6].active);
        assert_eq!(activity.active_windows[0].app_name, "code.exe");
        assert_eq!(activity.running_programs.len(), 1);
        assert_eq!(activity.end_reason, Some(EndReason::Interval));
        assert_eq!(activity.description.as_deref(), Some("Invoice export"));

        // Device context.
        assert_eq!(activity.platform.as_deref(), Some("windows"));
        assert_eq!(activity.agent_version.as_deref(), Some("9.9.9"));
        assert_eq!(activity.device_id.as_deref(), Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"));
        assert_eq!(activity.device_name.as_deref(), Some("DESKTOP-1"));
        assert_eq!(activity.os_version.as_deref(), Some("Windows 11 Pro 23H2 (10.0.22631)"));
        assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
        assert_eq!(activity.utc_offset_minutes, Some(360));
        // Stamped at send time, not here.
        assert!(activity.sent_at.is_none());
        assert!(uuid::Uuid::parse_str(&activity.client_activity_id).is_ok());
    }

    #[test]
    fn activity_is_not_measured_when_no_input_kind_is_recorded() {
        let perms = CapturePermissions { keyboard: false, mouse: false, ..all_allowed() };
        let mut measured = measurements();
        measured.key_presses.clear();
        measured.clicks.clear();
        measured.active_seconds.clear();
        let activity = build_activity(&block(&perms, None), measured, &device(), zone());
        assert_eq!(activity.productivity, None);
        assert_eq!(activity.idle_seconds, None);
        assert_eq!(activity.injected_input_events, None);
        assert!(activity.timeline.is_empty(), "all-inactive minutes would read as idle");
        // Apps are still recorded.
        assert!(!activity.active_windows.is_empty());
    }

    #[test]
    fn activity_is_not_measured_when_the_hooks_failed_during_the_block() {
        let perms = all_allowed();
        let measured = Measurements { input_available: false, ..measurements() };
        let activity = build_activity(&block(&perms, None), measured, &device(), zone());
        assert_eq!(activity.productivity, None);
        assert_eq!(activity.idle_seconds, None);
        assert!(activity.timeline.is_empty());
        // What was seen is still reported.
        assert_eq!(activity.keyboard_hits, 6);
    }

    #[test]
    fn the_screen_capture_follows_the_project_policy() {
        let perms = all_allowed();
        let measured = Measurements { screen: Some(capture(false)), ..measurements() };
        let activity = build_activity(&block(&perms, None), measured, &device(), zone());
        assert_eq!(activity.screenshot_png_base64.as_deref(), Some("/9j/screen"));
        assert_eq!(activity.screenshot_thumb_jpg_base64.as_deref(), Some("/9j/thumb"));
        assert_eq!(activity.screenshot_captured_at, Some(at(T0 + 250)));
        assert_eq!(activity.screenshot_blurred, Some(false));

        // Blur required: an unblurred capture is never sent.
        let blur = CapturePermissions { blur_screenshots: true, ..all_allowed() };
        let measured = Measurements { screen: Some(capture(false)), ..measurements() };
        let activity = build_activity(&block(&blur, None), measured, &device(), zone());
        assert!(!activity.has_captures());
        assert!(activity.screenshot_captured_at.is_none() && activity.screenshot_blurred.is_none());
        let measured = Measurements { screen: Some(capture(true)), ..measurements() };
        let activity = build_activity(&block(&blur, None), measured, &device(), zone());
        assert_eq!(activity.screenshot_blurred, Some(true));

        // Screenshots not allowed: dropped, whatever the caller passed.
        let none = CapturePermissions { screenshot: false, webcam: false, ..all_allowed() };
        let measured = Measurements { screen: Some(capture(true)), webcam_jpg_base64: Some("/9j/cam".into()), ..measurements() };
        let activity = build_activity(&block(&none, None), measured, &device(), zone());
        assert!(!activity.has_captures());
    }

    #[test]
    fn a_capture_taken_before_a_lock_is_kept() {
        // The random moment came before the lock; only a capture at the end
        // is forbidden.
        let perms = all_allowed();
        let spec = block(&perms, Some(EndReason::Lock));
        assert!(!spec.captures_at_end());
        let measured = Measurements { screen: Some(capture(false)), ..measurements() };
        let activity = build_activity(&spec, measured, &device(), zone());
        assert!(activity.screenshot_png_base64.is_some());
        assert_eq!(activity.end_reason, Some(EndReason::Lock));
        assert!(block(&perms, None).captures_at_end());
        assert!(block(&perms, Some(EndReason::Pause)).captures_at_end());
    }

    #[test]
    fn app_usage_follows_the_active_window_permission() {
        let perms = CapturePermissions { active_window: false, running_programs: false, ..all_allowed() };
        let measured = Measurements { slices: Vec::new(), running_programs: Vec::new(), ..measurements() };
        let activity = build_activity(&block(&perms, None), measured, &device(), zone());
        assert!(activity.active_windows.is_empty() && activity.running_programs.is_empty());
        assert!(activity.timeline.iter().all(|m| m.app_name.is_none()));

        // A block too short for a sample: the window in front stands in.
        let perms = all_allowed();
        let front = WindowInfo { app_name: "excel.exe".into(), window_title: "Q3".into(), ..Default::default() };
        let measured = Measurements { slices: Vec::new(), window_now: Some(front.clone()), ..measurements() };
        let activity = build_activity(&block(&perms, None), measured, &device(), zone());
        assert_eq!(activity.active_windows, vec![front]);
    }

    #[test]
    fn task_notes_are_trimmed_and_bounded() {
        assert_eq!(clean_description(None), None);
        assert_eq!(clean_description(Some("   ")), None);
        assert_eq!(clean_description(Some(" Fix login ")).as_deref(), Some("Fix login"));
        let long = "x".repeat(5000);
        assert_eq!(clean_description(Some(&long)).map(|d| d.chars().count()), Some(MAX_DESCRIPTION_CHARS));
    }

    #[test]
    fn spans_split_into_equal_pieces_of_at_most_one_interval() {
        let ten = TimeDelta::minutes(10);
        // 42 minutes: five pieces of 8 min 24 s, back to back.
        let pieces = split_span(at(T0), at(T0 + 42 * 60), ten);
        assert_eq!(pieces.len(), 5);
        assert_eq!(pieces[0].0, at(T0));
        assert_eq!(pieces[4].1, at(T0 + 42 * 60));
        assert!(pieces.windows(2).all(|w| w[0].1 == w[1].0), "no gaps");
        assert!(pieces.iter().all(|(s, e)| *e - *s <= ten && *e - *s >= TimeDelta::minutes(8)));
        // Exactly one interval: one piece. Under a minute: nothing.
        assert_eq!(split_span(at(T0), at(T0 + 600), ten).len(), 1);
        assert!(split_span(at(T0), at(T0 + 59), ten).is_empty());
        assert!(split_span(at(T0), at(T0 + 600), TimeDelta::zero()).is_empty());
    }

    #[test]
    fn kept_idle_time_is_recorded_as_idle_blocks() {
        let blocks = kept_idle_blocks(
            "p1",
            at(T0),
            at(T0 + 25 * 60),
            TimeDelta::minutes(10),
            Some("Client call"),
            &device(),
            Some("Asia/Dhaka"),
        );
        assert_eq!(blocks.len(), 3);
        for activity in &blocks {
            let length = (activity.ended_at - activity.started_at).num_seconds() as u32;
            assert!(length <= 600);
            assert_eq!(activity.productivity, Some(0));
            assert_eq!(activity.idle_seconds, Some(length));
            assert_eq!(activity.end_reason, Some(EndReason::Idle));
            assert!(activity.active_windows.is_empty() && activity.timeline.is_empty() && !activity.has_captures());
            assert_eq!(activity.description.as_deref(), Some("Client call"));
            assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
            assert_eq!(activity.device_id, device().device_id);
            assert!(activity.utc_offset_minutes.is_some());
        }
        assert_eq!(blocks[0].started_at, at(T0));
        assert_eq!(blocks[2].ended_at, at(T0 + 25 * 60));
        // Distinct blocks, distinct idempotency keys.
        assert_ne!(blocks[0].client_activity_id, blocks[1].client_activity_id);
        assert!(kept_idle_blocks("p1", at(T0), at(T0 + 30), TimeDelta::minutes(10), None, &device(), None).is_empty());
    }

    #[test]
    fn tracker_active_gate_controls_both_usage_and_input() {
        let (tracker, driver) = Tracker::new_fake();
        assert!(!tracker.is_tracking_active(), "Tracker must default to inactive (opt-in)");
        assert!(!driver.is_input_running());
        assert!(!driver.is_usage_running());

        tracker.set_tracking_active(true);
        assert!(tracker.is_tracking_active());
        assert!(driver.is_input_running());
        assert!(driver.is_usage_running());
        assert!(tracker.usage().is_active());
        assert!(tracker.input().is_active());

        tracker.set_tracking_active(false);
        assert!(!tracker.is_tracking_active());
        assert!(!driver.is_input_running());
        assert!(!driver.is_usage_running());
        assert!(!tracker.usage().is_active());
        assert!(!tracker.input().is_active());
    }

    #[test]
    fn tracker_per_sensor_permission_gating_with_fake_driver() {
        let (tracker, driver) = Tracker::new_fake();
        // Only active_window permitted
        let perms = CapturePermissions {
            keyboard: false,
            mouse: false,
            active_window: true,
            screenshot: false,
            webcam: false,
            running_programs: false,
            blur_screenshots: false,
        };
        tracker.apply_sensor_policy(true, Some(&perms));
        assert!(!driver.is_input_running(), "input hooks must not start when keyboard and mouse are denied");
        assert!(driver.is_usage_running(), "usage sampler should start when active_window is allowed");

        // Only keyboard permitted
        let perms2 = CapturePermissions {
            keyboard: true,
            mouse: false,
            active_window: false,
            ..perms
        };
        tracker.apply_sensor_policy(true, Some(&perms2));
        assert!(driver.is_input_running(), "input hooks should start when keyboard is allowed");
        assert!(!driver.is_usage_running(), "usage sampler must stop when active_window is denied");
        assert!(driver.last_keyboard_allowed());
        assert!(!driver.last_mouse_allowed());
    }

    #[test]
    fn tracker_consent_webcam_withdrawn_while_mouse_retained_stale_spec_zero_camera_calls() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        // 1. Initial grant with mouse and webcam
        let mut initial_perms = CapturePermissions::all_disabled();
        initial_perms.mouse = true;
        initial_perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&initial_perms));
        assert!(tracker.is_tracking_active());
        assert!(driver.is_input_running());

        // 2. User narrows consent: webcam is withdrawn, mouse is retained
        let mut narrowed_perms = initial_perms;
        narrowed_perms.webcam = false;
        tracker.apply_sensor_policy(true, Some(&narrowed_perms));
        assert!(tracker.is_tracking_active());
        assert!(driver.is_input_running());

        // 3. Stale block spec with webcam = true passed to snapshot
        let mut stale_perms = CapturePermissions::all_disabled();
        stale_perms.mouse = true;
        stale_perms.webcam = true; // STALE!
        let spec = block(&stale_perms, Some(EndReason::Interval));

        let activity = tracker.snapshot(&spec, None);

        // 4. Zero camera calls on acquisition provider, and webcam_jpg_base64 is None
        assert_eq!(acq.webcam_calls(), 0, "must NOT invoke camera when webcam is withdrawn in current effective policy");
        assert!(activity.webcam_jpg_base64.is_none());
    }

    #[test]
    fn tracker_consent_screenshot_withdrawn_direct_capture_screen_zero_calls() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        // 1. Initially active with screenshot
        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let cap = tracker.capture_screen(false);
        assert!(cap.is_some());
        assert_eq!(acq.screen_calls(), 1);

        // 2. User withdraws screenshot
        perms.screenshot = false;
        tracker.apply_sensor_policy(true, Some(&perms));

        // 3. Direct capture_screen call
        let cap_denied = tracker.capture_screen(false);
        assert!(cap_denied.is_none(), "capture_screen must return None when screenshot withdrawn");
        assert_eq!(acq.screen_calls(), 1, "provider screen_calls must remain 1 (zero new calls)");
    }

    #[test]
    fn tracker_consent_running_programs_withdrawn_active_window_retained_zero_process_calls() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        // 1. Initially active with active_window AND running_programs
        let mut initial_perms = CapturePermissions::all_disabled();
        initial_perms.active_window = true;
        initial_perms.running_programs = true;
        tracker.apply_sensor_policy(true, Some(&initial_perms));
        assert!(driver.is_usage_running());

        // 2. Narrowed: running_programs withdrawn, active_window retained
        let mut narrowed_perms = initial_perms;
        narrowed_perms.running_programs = false;
        tracker.apply_sensor_policy(true, Some(&narrowed_perms));
        assert!(driver.is_usage_running(), "usage driver still running for active_window");

        // 3. Stale block spec with running_programs = true
        let mut stale_perms = CapturePermissions::all_disabled();
        stale_perms.active_window = true;
        stale_perms.running_programs = true; // STALE!
        let spec = block(&stale_perms, Some(EndReason::Interval));

        let activity = tracker.snapshot(&spec, None);

        // 4. Zero process queries on acquisition provider
        assert_eq!(acq.process_calls(), 0, "must NOT query running programs when withdrawn in current effective policy");
        assert!(activity.running_programs.is_empty());
        // Meanwhile, window query was permitted because active_window was retained
        assert_eq!(acq.window_calls(), 1);
        assert!(!activity.active_windows.is_empty());
    }

    #[test]
    fn tracker_consent_inactive_or_paused_with_stale_spec_zero_acquisitions() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        // Tracking is stopped / paused / inactive (default or explicitly paused)
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());
        assert!(!driver.is_input_running());
        assert!(!driver.is_usage_running());

        // Stale block spec with everything allowed
        let stale_perms = CapturePermissions::default(); // all true
        let spec = block(&stale_perms, Some(EndReason::Pause));

        // Direct capture_screen attempt
        let cap = tracker.capture_screen(false);
        assert!(cap.is_none());
        assert_eq!(acq.screen_calls(), 0);

        // Direct current_app attempt
        let app = tracker.current_app();
        assert!(app.is_none());
        assert_eq!(acq.window_calls(), 0);

        // Direct snapshot attempt
        let activity = tracker.snapshot(&spec, None);
        assert_eq!(acq.webcam_calls(), 0, "0 webcam calls when inactive");
        assert_eq!(acq.window_calls(), 0, "0 window calls when inactive");
        assert_eq!(acq.process_calls(), 0, "0 process calls when inactive");
        assert!(activity.webcam_jpg_base64.is_none());
        assert!(activity.screenshot_png_base64.is_none());
        assert!(activity.running_programs.is_empty());
    }

    #[test]
    fn tracker_consent_legitimately_active_webcam_only_policy_invokes_camera_without_input_or_usage_drivers() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        // Webcam only active: input and active_window denied
        let mut webcam_only = CapturePermissions::all_disabled();
        webcam_only.webcam = true;
        tracker.apply_sensor_policy(true, Some(&webcam_only));

        // Tracking is active, but input and usage drivers are NOT running!
        assert!(tracker.is_tracking_active(), "tracking must be active for authorized webcam-only policy");
        assert!(!driver.is_input_running(), "input driver must NOT run");
        assert!(!driver.is_usage_running(), "usage driver must NOT run");

        let spec = block(&webcam_only, Some(EndReason::Interval));
        let activity = tracker.snapshot(&spec, None);

        // Synthetic camera provider WAS called!
        assert_eq!(acq.webcam_calls(), 1, "synthetic camera provider MUST be called for authorized webcam");
        assert!(activity.webcam_jpg_base64.is_some());
        assert_eq!(acq.window_calls(), 0);
        assert_eq!(acq.process_calls(), 0);
        assert_eq!(acq.screen_calls(), 0);
    }

    #[test]
    fn tracker_consent_held_block_safe_finalization_preserves_historical_accumulators_without_new_samples() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        // 1. Block ran while keyboard, mouse, and active_window were authorized
        let mut authorized_perms = CapturePermissions::all_disabled();
        authorized_perms.keyboard = true;
        authorized_perms.mouse = true;
        authorized_perms.active_window = true;
        authorized_perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&authorized_perms));

        // Record some historical input and usage slices during the block
        tracker.input().record_key_press(at(T0 + 10).timestamp_millis());
        tracker.input().record_click(at(T0 + 20).timestamp_millis());
        tracker.usage().record(at(T0 + 30).timestamp_millis(), Some(WindowInfo {
            app_name: "code.exe".into(),
            window_title: "code".into(),
            ..Default::default()
        }));
        tracker.usage().record(at(T0 + 60).timestamp_millis(), Some(WindowInfo {
            app_name: "code.exe".into(),
            window_title: "code".into(),
            ..Default::default()
        }));

        // 2. User revokes all consent before held block finalization
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());
        assert!(!driver.is_input_running());
        assert!(!driver.is_usage_running());

        // 3. Held block is finalized with its historical block perms
        let spec = block(&authorized_perms, Some(EndReason::Interval));
        let activity = tracker.snapshot(&spec, None);

        // 4. Assert NO new samples or captures were taken
        assert_eq!(acq.webcam_calls(), 0, "zero new webcam calls upon finalized held block after revocation");
        assert_eq!(acq.screen_calls(), 0, "zero new screen calls");
        assert_eq!(acq.window_calls(), 0, "zero new window queries");
        assert_eq!(acq.process_calls(), 0, "zero new process queries");

        // 5. But historical accumulators (keys, clicks, slices) are preserved!
        assert_eq!(activity.keyboard_hits, 1, "historical key presses preserved");
        assert_eq!(activity.mouse_clicks, 1, "historical clicks preserved");
        assert!(!activity.active_windows.is_empty(), "historical usage slices preserved");
        assert!(activity.webcam_jpg_base64.is_none());
    }

    #[test]
    fn tracker_consent_conservative_blur_applies_if_either_current_or_block_requires_blur() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let mut blur_perms = CapturePermissions::all_disabled();
        blur_perms.screenshot = true;
        blur_perms.blur_screenshots = true;
        tracker.apply_sensor_policy(true, Some(&blur_perms));

        // 1. capture_screen with blur=false must force blur=true because current_perms requires blur
        let cap = tracker.capture_screen(false);
        assert!(cap.is_some());
        assert!(acq.last_blur(), "capture_screen must pass blur=true to provider when policy requires blur");

        // 2. snapshot with unblurred capture must drop it when current_perms requires blur
        let unblurred_cap = prov_cap(&tracker, false);
        let spec = block(&blur_perms, Some(EndReason::Interval));
        let activity = tracker.snapshot(&spec, Some(unblurred_cap));
        assert!(activity.screenshot_png_base64.is_none(), "unblurred capture must be dropped when policy requires blur");

        // 3. snapshot with blurred capture is kept
        let blurred_cap = prov_cap(&tracker, true);
        let activity2 = tracker.snapshot(&spec, Some(blurred_cap));
        assert!(activity2.screenshot_png_base64.is_some());
        assert_eq!(activity2.screenshot_blurred, Some(true));

        // 4. snapshot with unblurred capture must drop it if historical block requires blur, even if current policy relaxes blur
        let mut no_blur_perms = blur_perms;
        no_blur_perms.blur_screenshots = false;
        tracker.apply_sensor_policy(true, Some(&no_blur_perms));
        let unblurred_cap2 = prov_cap(&tracker, false);
        let activity3 = tracker.snapshot(&spec, Some(unblurred_cap2));
        assert!(activity3.screenshot_png_base64.is_none(), "unblurred capture must be dropped when block requires blur even if current policy relaxed blur");
    }

    #[test]
    fn tracker_consent_snapshot_interleaving_revocation_prevents_new_webcam_admission() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        // 1. Initial grant with active_window and webcam
        let mut initial_perms = CapturePermissions::all_disabled();
        initial_perms.active_window = true;
        initial_perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&initial_perms));
        assert!(tracker.is_tracking_active());
        assert!(driver.is_usage_running());

        // 2. Set up deterministic channels to pause inside current_window
        let (window_entered_tx, window_entered_rx) = std::sync::mpsc::channel();
        let (revoke_done_tx, revoke_done_rx) = std::sync::mpsc::channel();
        let revoke_done_rx = Arc::new(Mutex::new(revoke_done_rx));
        let rx_for_hook = revoke_done_rx.clone();

        acq.set_on_current_window(move || {
            window_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for revocation");
        });

        // 3. Stale block spec with active_window and webcam
        let spec_perms = initial_perms;
        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, None)
        });

        // 4. Wait for snapshot to enter current_window
        window_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for current_window entry");

        // 5. Concurrently revoke all consent while snapshot is paused in current_window
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());
        assert!(!driver.is_usage_running());

        // 6. Release current_window so snapshot can proceed
        revoke_done_tx.send(()).unwrap();

        // 7. Await snapshot completion
        let activity = handle.join().expect("snapshot thread panicked");

        // 8. In-flight window call completed, but was discarded on exit because revocation completed in flight
        assert_eq!(acq.window_calls(), 1);
        assert!(activity.active_windows.is_empty(), "in-flight window result must be dropped upon revocation");

        // 9. PROVE ZERO NEW WEBCAM ADMISSIONS: capture_webcam must NEVER be admitted after withdrawal!
        assert_eq!(acq.webcam_calls(), 0, "must NOT admit webcam call after withdrawal completed");
        assert!(activity.webcam_jpg_base64.is_none());
        assert_eq!(acq.process_calls(), 0);
        assert_eq!(acq.screen_calls(), 0);
    }

    #[test]
    fn tracker_consent_process_only_positive_regression() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();

        // 1. Process-only active policy: running_programs = true, active_window/input denied
        let mut process_only = CapturePermissions::all_disabled();
        process_only.running_programs = true;
        tracker.apply_sensor_policy(true, Some(&process_only));

        // 2. Tracking is active, but usage and input drivers remain strictly stopped
        assert!(tracker.is_tracking_active(), "tracker must be active for process-only grant");
        assert!(!driver.is_usage_running(), "usage driver must NOT run when active_window is denied");
        assert!(!driver.is_input_running(), "input driver must NOT run when input is denied");

        // 3. Snapshot with block permitting running_programs
        let spec = block(&process_only, Some(EndReason::Interval));
        let activity = tracker.snapshot(&spec, None);

        // 4. Exactly one synthetic process query on provider; zero window/camera/screen calls
        assert_eq!(acq.process_calls(), 1, "exactly one process query admitted");
        assert_eq!(acq.window_calls(), 0, "zero window calls");
        assert_eq!(acq.webcam_calls(), 0, "zero webcam calls");
        assert_eq!(acq.screen_calls(), 0, "zero screen calls");

        // 5. Returned fixture is present in emitted activity
        assert_eq!(activity.running_programs.len(), 1);
        assert_eq!(activity.running_programs[0].app_name, "code.exe");
        assert!(activity.active_windows.is_empty());
    }

    #[test]
    fn tracker_consent_capture_screen_race_and_revocation_prevents_subsequent_admission() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let (screen_entered_tx, screen_entered_rx) = std::sync::mpsc::channel();
        let (revoke_done_tx, revoke_done_rx) = std::sync::mpsc::channel();
        let revoke_done_rx = Arc::new(Mutex::new(revoke_done_rx));
        let rx_for_hook = revoke_done_rx.clone();

        acq.set_on_capture_screen(move |_blur| {
            screen_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for revocation");
        });

        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            tracker_clone.capture_screen(false)
        });

        // Wait until provider is entered
        screen_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for capture_screen entry");

        // Revoke screenshot capability while in-flight
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());

        // Release in-flight capture
        revoke_done_tx.send(()).unwrap();

        let cap = handle.join().expect("thread panicked");
        // In-flight capture completed on hardware/provider, but post-admission verification discards it
        assert!(cap.is_none(), "in-flight capture must be discarded if revoked while in flight");
        assert_eq!(acq.screen_calls(), 1);

        // Subsequent call to capture_screen: rejected at admission (zero new provider calls)
        let second_cap = tracker.capture_screen(false);
        assert!(second_cap.is_none());
        assert_eq!(acq.screen_calls(), 1, "zero subsequent admissions after withdrawal");
    }

    #[test]
    fn tracker_consent_pause_and_partial_narrowing_during_snapshot() {
        let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut initial_perms = CapturePermissions::all_disabled();
        initial_perms.active_window = true;
        initial_perms.running_programs = true;
        initial_perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&initial_perms));

        let (window_entered_tx, window_entered_rx) = std::sync::mpsc::channel();
        let (narrow_done_tx, narrow_done_rx) = std::sync::mpsc::channel();
        let narrow_done_rx = Arc::new(Mutex::new(narrow_done_rx));
        let rx_for_hook = narrow_done_rx.clone();

        acq.set_on_current_window(move || {
            window_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for narrowing");
        });

        let spec_perms = initial_perms;
        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, None)
        });

        window_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout");

        // Partial narrowing: revoke active_window and webcam, but RETAIN running_programs
        let mut narrowed = CapturePermissions::all_disabled();
        narrowed.running_programs = true;
        tracker.apply_sensor_policy(true, Some(&narrowed));
        assert!(!driver.is_usage_running(), "usage driver stopped when active_window revoked");

        narrow_done_tx.send(()).unwrap();

        let activity = handle.join().expect("snapshot thread panicked");

        // 1. In-flight window call was discarded
        assert_eq!(acq.window_calls(), 1);
        assert!(activity.active_windows.is_empty());

        // 2. Retained running_programs capability WAS admitted!
        assert_eq!(acq.process_calls(), 1, "retained running_programs must be queried");
        assert_eq!(activity.running_programs.len(), 1);

        // 3. Withdrawn webcam was REJECTED at admission (zero calls!)
        assert_eq!(acq.webcam_calls(), 0, "withdrawn webcam must not be called");
        assert!(activity.webcam_jpg_base64.is_none());
    }

    #[test]
    fn tracker_consent_no_deadlock_between_slow_provider_and_sensor_policy() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.active_window = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_current_window(move || {
            entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            tracker_clone.current_app()
        });

        entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout");

        // Crucial test: apply_sensor_policy must NOT block while current_window is executing!
        // We assert apply_sensor_policy completes immediately without deadlocking or waiting for release.
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());

        // Now release the provider
        release_tx.send(()).unwrap();
        let app = handle.join().expect("thread panicked");
        assert!(app.is_none(), "result discarded because revoked while in flight");
    }

    #[test]
    fn tracker_consent_capture_screen_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let (screen_entered_tx, screen_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_screen(move |_blur| {
            screen_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            tracker_clone.capture_screen(false)
        });

        // 1. Wait until provider is entered
        screen_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for capture_screen entry");

        // 2. Revoke screenshot capability while in-flight
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());

        // 3. Regrant screenshot capability BEFORE releasing the old provider
        tracker.apply_sensor_policy(true, Some(&perms));
        assert!(tracker.is_tracking_active());

        // 4. Release old in-flight provider
        release_tx.send(()).unwrap();

        let old_cap = handle.join().expect("thread panicked");
        // 5. Assert: old capture rejected despite current policy having screenshot = true (ABA immunity)
        assert!(old_cap.is_none(), "old capture from invalidated consent interval must be rejected after regrant");
        assert_eq!(acq.screen_calls(), 1);

        // 6. Assert: genuinely fresh post-regrant acquisition succeeds!
        acq.set_on_capture_screen(|_| {});
        let fresh_cap = tracker.capture_screen(false);
        assert!(fresh_cap.is_some(), "fresh capture admitted and validated under current generation");
        assert_eq!(acq.screen_calls(), 2);
    }

    #[test]
    fn tracker_consent_webcam_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);
        acq.set_canned_webcam(Some("valid_fake_webcam_data".into()));

        let mut perms = CapturePermissions::all_disabled();
        perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_webcam(move || {
            webcam_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, None)
        });

        // 1. Wait until provider is entered
        webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

        // 2. Revoke webcam capability while in-flight
        tracker.apply_sensor_policy(false, None);
        assert!(!tracker.is_tracking_active());

        // 3. Regrant webcam capability BEFORE releasing the old provider
        tracker.apply_sensor_policy(true, Some(&perms));
        assert!(tracker.is_tracking_active());

        // 4. Release old in-flight provider
        release_tx.send(()).unwrap();

        let activity = handle.join().expect("thread panicked");
        // 5. Assert: old webcam capture rejected despite current policy having webcam = true (ABA immunity)
        assert!(activity.webcam_jpg_base64.is_none(), "old webcam capture from invalidated interval must be rejected");
        assert_eq!(acq.webcam_calls(), 1);

        // 6. Assert: genuinely fresh post-regrant acquisition succeeds!
        acq.set_on_capture_webcam(|| {});
        let fresh_spec = block(&perms, Some(EndReason::Interval));
        let fresh_activity = tracker.snapshot(&fresh_spec, None);
        assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam capture admitted and validated under current generation");
        assert_eq!(acq.webcam_calls(), 2);
    }

    #[test]
    fn tracker_consent_current_app_and_snapshot_window_revocation_regrant_aba_rejected() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);
        acq.set_canned_window(Some(WindowInfo {
            app_name: "test_editor.exe".into(),
            window_title: "test editor".into(),
            ..Default::default()
        }));

        let mut perms = CapturePermissions::all_disabled();
        perms.active_window = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        // Part A: current_app ABA test
        {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let release_rx = Arc::new(Mutex::new(release_rx));
            let rx_for_hook = release_rx.clone();

            acq.set_on_current_window(move || {
                entered_tx.send(()).unwrap();
                rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
            });

            let tracker_clone = tracker.clone();
            let handle = std::thread::spawn(move || {
                tracker_clone.current_app()
            });

            entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for current_app entry");

            // ABA transition: revoke then regrant before releasing provider
            tracker.apply_sensor_policy(false, None);
            tracker.apply_sensor_policy(true, Some(&perms));

            release_tx.send(()).unwrap();
            let app = handle.join().expect("thread panicked");
            assert!(app.is_none(), "old current_app result from invalidated consent interval must be rejected");
        }

        // Part B: snapshot current_window ABA test
        {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let release_rx = Arc::new(Mutex::new(release_rx));
            let rx_for_hook = release_rx.clone();

            acq.set_on_current_window(move || {
                entered_tx.send(()).unwrap();
                rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
            });

            let tracker_clone = tracker.clone();
            let spec_perms = perms;
            let handle = std::thread::spawn(move || {
                let spec = block(&spec_perms, Some(EndReason::Interval));
                tracker_clone.snapshot(&spec, None)
            });

            entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for snapshot window entry");

            // ABA transition: revoke then regrant before releasing provider
            tracker.apply_sensor_policy(false, None);
            tracker.apply_sensor_policy(true, Some(&perms));

            release_tx.send(()).unwrap();
            let activity = handle.join().expect("thread panicked");
            assert!(activity.active_windows.is_empty(), "old window from invalidated consent interval must not be recorded");
        }

        // Part C: fresh current_app succeeds under current generation
        acq.set_on_current_window(|| {});
        let fresh_app = tracker.current_app();
        assert_eq!(fresh_app.as_deref(), Some("test_editor.exe"));
    }

    #[test]
    fn tracker_consent_running_programs_revocation_regrant_aba_rejected() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);
        acq.set_canned_processes(Some(vec![WindowInfo {
            app_name: "proc_worker.exe".into(),
            window_title: "proc worker".into(),
            ..Default::default()
        }]));

        let mut perms = CapturePermissions::all_disabled();
        perms.running_programs = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_running_programs(move || {
            entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, None)
        });

        entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for running_programs entry");

        // ABA transition: revoke then regrant before releasing provider
        tracker.apply_sensor_policy(false, None);
        tracker.apply_sensor_policy(true, Some(&perms));

        release_tx.send(()).unwrap();
        let activity = handle.join().expect("thread panicked");
        assert!(activity.running_programs.is_empty(), "old processes from invalidated interval must be rejected");

        // Fresh acquisition succeeds
        acq.set_on_running_programs(|| {});
        let fresh_spec = block(&perms, Some(EndReason::Interval));
        let fresh_activity = tracker.snapshot(&fresh_spec, None);
        assert_eq!(fresh_activity.running_programs.len(), 1);
        assert_eq!(fresh_activity.running_programs[0].app_name, "proc_worker.exe");
    }

    #[test]
    fn tracker_consent_final_publication_rejects_earlier_results_after_later_provider_aba() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        // 1. Initial grant authorizing all sensors: keyboard, mouse, active_window, running_programs, screenshot, webcam
        let mut perms = CapturePermissions::all_disabled();
        perms.keyboard = true;
        perms.mouse = true;
        perms.active_window = true;
        perms.running_programs = true;
        perms.screenshot = true;
        perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        // 2. Populate genuinely authorized historical accumulators during the block [T0, T0+600]
        tracker.input().record_key_press(at(T0 + 10).timestamp_millis());
        tracker.input().record_click(at(T0 + 20).timestamp_millis());
        tracker.usage().record(
            at(T0 + 10).timestamp_millis(),
            Some(WindowInfo {
                app_name: "historical_app.exe".into(),
                window_title: "historical title".into(),
                ..Default::default()
            }),
        );
        tracker.usage().record(
            at(T0 + 20).timestamp_millis(),
            Some(WindowInfo {
                app_name: "historical_app.exe".into(),
                window_title: "historical title".into(),
                ..Default::default()
            }),
        );
        let historical_slice_count_before = tracker.usage().slices_between(at(T0).timestamp_millis(), at(T0 + 600).timestamp_millis()).len();
        assert_eq!(historical_slice_count_before, 1, "must have 1 authorized historical usage slice");

        // 3. Gather earlier screen capture under current generation
        let prov_cap = tracker.capture_screen(false);
        assert!(prov_cap.is_some(), "capture_screen under initial grant must succeed");

        // 4. Configure acquisition provider for window, processes, and webcam
        acq.set_canned_window(Some(WindowInfo {
            app_name: "early_app.exe".into(),
            window_title: "early title".into(),
            ..Default::default()
        }));
        acq.set_canned_processes(Some(vec![WindowInfo {
            app_name: "early_proc.exe".into(),
            window_title: "proc title".into(),
            ..Default::default()
        }]));

        // 5. Block inside webcam acquisition (which runs AFTER window and process queries)
        let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_webcam(move || {
            webcam_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let prov_cap_clone = prov_cap.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, prov_cap_clone)
        });

        // Wait until webcam provider is entered (meaning window, process, and screen checks are staged)
        webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

        // 6. ABA transition: revoke then re-grant all capabilities BEFORE releasing webcam
        tracker.apply_sensor_policy(false, None);
        tracker.apply_sensor_policy(true, Some(&perms));

        // 7. Release webcam provider
        release_tx.send(()).unwrap();

        let activity = handle.join().expect("thread panicked");

        // 8. Prove invalidated NEW values are excluded at final activity assembly:
        assert!(activity.running_programs.is_empty(), "early running programs collected before webcam block must be rejected at final publication boundary");
        assert!(activity.screenshot_png_base64.is_none(), "early screenshot from invalidated consent interval must be rejected at final publication boundary");
        assert!(activity.webcam_jpg_base64.is_none(), "webcam captured in invalidated interval must be rejected");

        // 9. Prove usage is NOT contaminated:
        // The invalidated window sample from ended_at must NOT have been recorded via usage.record()
        let historical_slices_after = tracker.usage().slices_between(at(T0).timestamp_millis(), at(T0 + 600).timestamp_millis());
        assert_eq!(historical_slices_after.len(), historical_slice_count_before, "usage.record must NOT record invalidated window sample at block end");
        assert!(
            historical_slices_after.iter().all(|s| s.app != "early_app.exe"),
            "historical usage must not be contaminated with early_app.exe from invalidated consent interval"
        );

        // 10. Prove genuinely authorized historical accumulators ARE preserved:
        assert_eq!(activity.keyboard_hits, 1, "genuinely authorized historical keyboard events must be preserved");
        assert_eq!(activity.mouse_clicks, 1, "genuinely authorized historical mouse clicks must be preserved");
        assert_eq!(activity.active_windows.len(), 1, "active_windows from authorized historical slices must be preserved");
        assert_eq!(activity.active_windows[0].app_name, "historical_app.exe");
        assert!(activity.active_windows.iter().all(|w| w.app_name != "early_app.exe"), "early_app.exe must not appear in active_windows");

        // 11. Prove fresh post-regrant behavior:
        acq.set_on_capture_webcam(|| {});
        let fresh_prov_cap = tracker.capture_screen(false);
        assert!(fresh_prov_cap.is_some(), "fresh capture_screen under post-regrant generation succeeds");
        let fresh_spec = block(&perms, Some(EndReason::Interval));
        let fresh_activity = tracker.snapshot(&fresh_spec, fresh_prov_cap);
        assert!(fresh_activity.screenshot_png_base64.is_some(), "fresh screenshot accepted under current generation");
        assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam accepted under current generation");
        assert_eq!(fresh_activity.running_programs.len(), 1, "fresh running programs accepted under current generation");
        assert_eq!(fresh_activity.running_programs[0].app_name, "early_proc.exe");
    }

    #[test]
    fn tracker_consent_screenshot_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
        let (tracker, _driver, _acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        // 1. Take capture_screen under initial grant
        let prov_cap = tracker.capture_screen(false);
        assert!(prov_cap.is_some());

        // 2. Revoke and regrant screenshot permission (ABA transition)
        tracker.apply_sensor_policy(false, None);
        tracker.apply_sensor_policy(true, Some(&perms));

        // 3. Snapshot with old prov_cap must reject the screenshot due to generation mismatch
        let spec = block(&perms, Some(EndReason::Interval));
        let activity = tracker.snapshot(&spec, prov_cap);
        assert!(activity.screenshot_png_base64.is_none(), "old screenshot from invalidated consent interval must be rejected");

        // 4. Fresh capture_screen under new generation succeeds
        let fresh_prov_cap = tracker.capture_screen(false);
        assert!(fresh_prov_cap.is_some());
        let activity2 = tracker.snapshot(&spec, fresh_prov_cap);
        assert!(activity2.screenshot_png_base64.is_some(), "fresh screenshot under current generation succeeds");
    }

    #[test]
    fn tracker_consent_screenshot_rejected_after_later_webcam_aba_revocation() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);
        acq.set_canned_webcam(Some("/9j/valid_webcam_during_blocked_screen_aba".into()));

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let prov_cap = tracker.capture_screen(false);
        assert!(prov_cap.is_some());

        // Block inside webcam acquisition (running after screenshot handoff)
        let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_webcam(move || {
            webcam_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let prov_cap_clone = prov_cap.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, prov_cap_clone)
        });

        webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

        // Isolated ABA transition: withdraw ONLY screenshot while keeping active=true and webcam=true
        let mut perms_webcam_only = perms;
        perms_webcam_only.screenshot = false;
        tracker.apply_sensor_policy(true, Some(&perms_webcam_only));
        assert!(tracker.is_tracking_active());

        // Regrant screenshot BEFORE releasing webcam
        tracker.apply_sensor_policy(true, Some(&perms));
        assert!(tracker.is_tracking_active());

        release_tx.send(()).unwrap();
        let activity = handle.join().expect("thread panicked");

        // Old screenshot from invalidated interval must be rejected
        assert!(activity.screenshot_png_base64.is_none(), "screenshot associated with revoked/ABA interval must be rejected at publication");
        // Continuously authorized webcam result must be present!
        assert!(activity.webcam_jpg_base64.is_some(), "continuously authorized webcam capture must be kept");
        assert_eq!(activity.webcam_jpg_base64.as_deref(), Some("/9j/valid_webcam_during_blocked_screen_aba"));

        // Assert exact provider call counts
        assert_eq!(acq.screen_calls(), 1, "exactly 1 screen call from earlier capture_screen");
        assert_eq!(acq.webcam_calls(), 1, "exactly 1 webcam call during snapshot");

        // Assert fresh screenshot under current generation succeeds
        acq.set_on_capture_webcam(|| {});
        let fresh_prov_cap = tracker.capture_screen(false);
        assert!(fresh_prov_cap.is_some(), "fresh capture_screen under current generation succeeds");
        let fresh_spec = block(&perms, Some(EndReason::Interval));
        let fresh_activity = tracker.snapshot(&fresh_spec, fresh_prov_cap);
        assert!(fresh_activity.screenshot_png_base64.is_some(), "fresh screenshot under current generation accepted");
        assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam accepted");
        assert_eq!(acq.screen_calls(), 2);
        assert_eq!(acq.webcam_calls(), 2);
    }

    #[test]
    fn tracker_consent_screenshot_blur_tightening_during_blocked_webcam() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        perms.blur_screenshots = false;
        perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        // Take unblurred capture under initial policy
        let prov_cap = tracker.capture_screen(false);
        assert!(prov_cap.as_ref().is_some_and(|c| !c.capture.blurred));

        let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_webcam(move || {
            webcam_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let prov_cap_clone = prov_cap.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, prov_cap_clone)
        });

        webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

        // Tighten policy to require blur while webcam is blocked
        let mut tight_perms = perms;
        tight_perms.blur_screenshots = true;
        tracker.apply_sensor_policy(true, Some(&tight_perms));

        release_tx.send(()).unwrap();
        let activity = handle.join().expect("thread panicked");

        assert!(activity.screenshot_png_base64.is_none(), "unblurred screenshot must be dropped when policy blur requirement is tightened during snapshot execution");
    }

    #[test]
    fn tracker_consent_unchanged_policy_republish_does_not_invalidate_in_flight_acquisition() {
        let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
        let tracker = Arc::new(tracker);

        let mut perms = CapturePermissions::all_disabled();
        perms.screenshot = true;
        perms.active_window = true;
        perms.running_programs = true;
        perms.webcam = true;
        tracker.apply_sensor_policy(true, Some(&perms));

        let gen_initial = tracker.auth.read().unwrap().generations;

        // Take screen capture under initial generation
        let prov_cap = tracker.capture_screen(false);
        assert!(prov_cap.is_some());
        assert_eq!(prov_cap.as_ref().unwrap().generation, gen_initial.screen);

        // Block inside webcam acquisition during snapshot
        let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_capture_webcam(move || {
            webcam_entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let prov_cap_clone = prov_cap.clone();
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, prov_cap_clone)
        });

        webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

        // Republish IDENTICAL policy (simulating routine worker publish tick)
        tracker.apply_sensor_policy(true, Some(&perms));

        let gen_republished = tracker.auth.read().unwrap().generations;
        assert_eq!(gen_initial, gen_republished, "generations must NOT advance on identical policy republish");

        release_tx.send(()).unwrap();
        let activity = handle.join().expect("thread panicked");

        // In-flight acquisitions must NOT be falsely invalidated by unchanged policy republish
        assert!(activity.screenshot_png_base64.is_some(), "screenshot must be kept on unchanged policy republish");
        assert!(activity.webcam_jpg_base64.is_some(), "webcam must be kept on unchanged policy republish");
    }
}


