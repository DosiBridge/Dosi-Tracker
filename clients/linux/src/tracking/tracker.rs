//! The Linux `Tracker`: owns the consent-gated sensors of the session and
//! builds each finished block's measurements.
//!
//! It is the production [`Machine`] of the worker. Every OS adapter gets the
//! explicit display or bus address of the probed session
//! ([`super::session_probe::SessionInfo`]); on Wayland no X11 adapter is ever
//! constructed (design 4).
//!
//! - One `Arc<Mutex<TrackerLifecycle>>` is shared with the
//!   [`ConsentGatedTracker`] (active window, running programs) and the
//!   capture thread: [`Tracker::apply_sensor_policy`] configures it with the
//!   effective permissions and starts or stops it, so every change bumps the
//!   generation and in-flight samples are discarded. Time-only tracking
//!   (consent granted, every capability off) leaves it `Stopped` while the
//!   worker still records blocks.
//! - The [`SensorDriver`] (ported from Windows `tracking/lifecycle.rs`) starts
//!   and stops the XI2 listener and the usage sampler synchronously; the
//!   native one is X11-only, Wayland gets [`NullSensorDriver`].
//! - Idle: the XI2 log's last physical input while the listener measures,
//!   else the session's idle source (MIT-SCREEN-SAVER on X11, Mutter /
//!   ScreenSaver / logind on Wayland); with none the session is in manual
//!   mode and the user always counts as present.
//! - Screenshots run on `dosi-capture` (X11 GetImage or the portal), with a
//!   lifecycle ticket acquired before and validated after; the X11 one can
//!   also be taken synchronously at a block's end (bounded to 3 s).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc::UnboundedSender;

use super::build::{
    build_activity, kept_idle_blocks, BlockSpec, BuiltBlock, Measurements, ProvenanceScreenCapture,
    Zone,
};
use super::capture::encode_frame;
use super::dbus_idle::{probe_idle_sources, IdleBuses};
use super::gated::ConsentGatedTracker;
use super::input::InputLog;
use super::portal_screen::PortalScreenSource;
use super::procfs::{ProcFsRunningAppsProvider, ProgramFilter, SysProcFsReader};
use super::session_probe::SessionInfo;
use super::system::DeviceInfo;
use super::traits::{
    ActiveWindowProvider, Availability as SourceAvailability, CaptureTarget, IdleSource,
    RunningAppsProvider, ScreenSource, TrackingError,
};
use super::usage::{UsageRecorder, UsageSampler, SAMPLE_EVERY};
use super::wayland::WaylandActiveWindowProvider;
use super::x11::X11ActiveWindowProvider;
use super::x11_idle::X11ScreenSaverIdleSource;
use super::x11_input::{InputKinds, X11InputListener};
use super::x11_screen::X11ScreenSource;
use crate::agent::blocks::BlockId;
use crate::agent::model::Activity;
use crate::agent::state::{PlatformAvailability, SessionKind, SessionSummary};
use crate::agent::worker::{CaptureEvent, Machine, PlatformCapabilities, PlatformCapability};
use crate::consent::ConsentedPermissions;
use crate::lifecycle::{AcquisitionCapability, TrackerLifecycle, TrackerState};

/// The longest a synchronous capture at a block's end may take.
pub const CAPTURE_AT_END_TIMEOUT: Duration = Duration::from_secs(3);

/// A sensor that failed to start is tried again this often (not on every
/// 2 s publish).
pub const SENSOR_RETRY: Duration = Duration::from_secs(30);

/// The XI2 listener starts right after the block it measures (on the same
/// publish); input in its first moments is not lost to a straddled second.
pub const INPUT_START_GRACE: TimeDelta = TimeDelta::seconds(2);

const WAYLAND_UNSUPPORTED: &str =
    "Not available on Wayland in this version: the compositor does not share it.";
const NO_SESSION: &str = "No graphical session is attached.";
const NO_PORTAL: &str = "This desktop offers no screenshot portal.";

/// The hardware/OS sensor lifecycle (ported from Windows `SensorDriver`).
/// Every call is synchronous: when `stop_*` returns, the thread is gone.
pub trait SensorDriver: Send + Sync {
    /// Start counting `kinds` into `log` (restarting when the kinds change).
    fn start_input(&self, log: Arc<InputLog>, kinds: InputKinds) -> Result<(), TrackingError>;
    /// Stop counting; input is "not measured" from now on.
    fn stop_input(&self);
    fn is_input_running(&self) -> bool;
    /// Start sampling the active window through `provider` into `recorder`.
    fn start_usage(
        &self,
        recorder: Arc<UsageRecorder>,
        provider: Arc<dyn ActiveWindowProvider>,
    ) -> Result<(), TrackingError>;
    fn stop_usage(&self);
    fn is_usage_running(&self) -> bool;
}

/// The X11 sensors: the XI2 listener (`dosi-xi2`) and the usage sampler
/// (`dosi-usage`) on the explicit display.
pub struct NativeX11SensorDriver {
    display: String,
    sample_every: Duration,
    input: Mutex<Option<(X11InputListener, InputKinds)>>,
    usage: Mutex<Option<UsageSampler>>,
}

impl NativeX11SensorDriver {
    pub fn new(display: impl Into<String>) -> Self {
        Self::with_sample_every(display, SAMPLE_EVERY)
    }

    /// With another sampling period (the harness tests).
    pub fn with_sample_every(display: impl Into<String>, sample_every: Duration) -> Self {
        Self {
            display: display.into(),
            sample_every,
            input: Mutex::new(None),
            usage: Mutex::new(None),
        }
    }
}

impl SensorDriver for NativeX11SensorDriver {
    fn start_input(&self, log: Arc<InputLog>, kinds: InputKinds) -> Result<(), TrackingError> {
        let mut slot = self.input.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((listener, running)) = slot.as_ref() {
            if *running == kinds && listener.is_running() {
                return Ok(());
            }
        }
        // Other kinds (narrowed or widened) or a dead listener: stop first.
        if let Some((listener, _)) = slot.take() {
            if let Err(e) = listener.stop() {
                tracing::warn!(error = %e, "the XI2 listener did not stop cleanly");
            }
        }
        let listener = X11InputListener::start(&self.display, log, kinds)?;
        *slot = Some((listener, kinds));
        Ok(())
    }

    fn stop_input(&self) {
        let taken = self.input.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((listener, _)) = taken {
            if let Err(e) = listener.stop() {
                tracing::warn!(error = %e, "the XI2 listener did not stop cleanly");
            }
        }
    }

    fn is_input_running(&self) -> bool {
        self.input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|(listener, _)| listener.is_running())
    }

    fn start_usage(
        &self,
        recorder: Arc<UsageRecorder>,
        provider: Arc<dyn ActiveWindowProvider>,
    ) -> Result<(), TrackingError> {
        let mut slot = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(UsageSampler::start(recorder, provider, self.sample_every)?);
        }
        Ok(())
    }

    fn stop_usage(&self) {
        let taken = self.usage.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(sampler) = taken {
            if let Err(e) = sampler.stop() {
                tracing::warn!(error = %e, "the usage sampler did not stop cleanly");
            }
        }
    }

    fn is_usage_running(&self) -> bool {
        self.usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

/// The sensors of a session that has none the agent may use (Wayland in v1,
/// no session): nothing ever starts.
#[derive(Debug, Default)]
pub struct NullSensorDriver;

impl SensorDriver for NullSensorDriver {
    fn start_input(&self, _log: Arc<InputLog>, _kinds: InputKinds) -> Result<(), TrackingError> {
        Err(TrackingError::UnsupportedSession(
            WAYLAND_UNSUPPORTED.into(),
        ))
    }

    fn stop_input(&self) {}

    fn is_input_running(&self) -> bool {
        false
    }

    fn start_usage(
        &self,
        _recorder: Arc<UsageRecorder>,
        _provider: Arc<dyn ActiveWindowProvider>,
    ) -> Result<(), TrackingError> {
        Err(TrackingError::UnsupportedSession(
            WAYLAND_UNSUPPORTED.into(),
        ))
    }

    fn stop_usage(&self) {}

    fn is_usage_running(&self) -> bool {
        false
    }
}

/// A sensor driver for synthetic tests: no OS side effects, counters only.
/// Starting input marks the log as measured (as the XI2 listener does).
#[derive(Default)]
pub struct FakeSensorDriver {
    input_started: AtomicUsize,
    input_stopped: AtomicUsize,
    usage_started: AtomicUsize,
    usage_stopped: AtomicUsize,
    input_running: AtomicBool,
    usage_running: AtomicBool,
    last_keyboard: AtomicBool,
    last_mouse: AtomicBool,
    fail_input: AtomicBool,
    input_attempts: AtomicU32,
    log: Mutex<Option<Arc<InputLog>>>,
}

impl FakeSensorDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn input_started_count(&self) -> usize {
        self.input_started.load(Ordering::SeqCst)
    }

    pub fn input_stopped_count(&self) -> usize {
        self.input_stopped.load(Ordering::SeqCst)
    }

    pub fn usage_started_count(&self) -> usize {
        self.usage_started.load(Ordering::SeqCst)
    }

    pub fn usage_stopped_count(&self) -> usize {
        self.usage_stopped.load(Ordering::SeqCst)
    }

    pub fn last_keyboard_allowed(&self) -> bool {
        self.last_keyboard.load(Ordering::SeqCst)
    }

    pub fn last_mouse_allowed(&self) -> bool {
        self.last_mouse.load(Ordering::SeqCst)
    }

    /// Make every input start fail (an X server without XI2).
    pub fn set_fail_input(&self, fail: bool) {
        self.fail_input.store(fail, Ordering::SeqCst);
    }

    pub fn input_attempts(&self) -> u32 {
        self.input_attempts.load(Ordering::SeqCst)
    }
}

impl SensorDriver for FakeSensorDriver {
    fn start_input(&self, log: Arc<InputLog>, kinds: InputKinds) -> Result<(), TrackingError> {
        self.input_attempts.fetch_add(1, Ordering::SeqCst);
        if self.fail_input.load(Ordering::SeqCst) {
            return Err(TrackingError::UnsupportedSession("no XInput 2.2".into()));
        }
        self.last_keyboard.store(kinds.keyboard, Ordering::SeqCst);
        self.last_mouse.store(kinds.mouse, Ordering::SeqCst);
        if !self.input_running.swap(true, Ordering::SeqCst) {
            self.input_started.fetch_add(1, Ordering::SeqCst);
            log.mark_healthy(Utc::now());
            *self.log.lock().unwrap_or_else(|e| e.into_inner()) = Some(log);
        }
        Ok(())
    }

    fn stop_input(&self) {
        if self.input_running.swap(false, Ordering::SeqCst) {
            self.input_stopped.fetch_add(1, Ordering::SeqCst);
            if let Some(log) = self.log.lock().unwrap_or_else(|e| e.into_inner()).take() {
                log.mark_unhealthy(Utc::now());
            }
        }
    }

    fn is_input_running(&self) -> bool {
        self.input_running.load(Ordering::SeqCst)
    }

    fn start_usage(
        &self,
        _recorder: Arc<UsageRecorder>,
        _provider: Arc<dyn ActiveWindowProvider>,
    ) -> Result<(), TrackingError> {
        if !self.usage_running.swap(true, Ordering::SeqCst) {
            self.usage_started.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn stop_usage(&self) {
        if self.usage_running.swap(false, Ordering::SeqCst) {
            self.usage_stopped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn is_usage_running(&self) -> bool {
        self.usage_running.load(Ordering::SeqCst)
    }
}

/// The session's screen source.
#[derive(Clone)]
pub struct ScreenParts {
    pub source: Arc<dyn ScreenSource>,
    /// `x11` or `portal`, for the capability snapshot.
    pub backend: &'static str,
    /// May it capture synchronously at a block's end (X11 GetImage)? The
    /// portal may show a dialog, so it never does.
    pub at_block_end: bool,
    /// Ask again after the user refused (the portal's `reset`).
    pub reset: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Everything a [`Tracker`] is built from (the native adapters, or fakes).
pub struct TrackerParts {
    pub kind: SessionKind,
    pub desktop: Option<String>,
    pub lifecycle: Arc<Mutex<TrackerLifecycle>>,
    pub device: DeviceInfo,
    pub driver: Arc<dyn SensorDriver>,
    pub active_window: Arc<dyn ActiveWindowProvider>,
    pub running_apps: Arc<dyn RunningAppsProvider>,
    pub screen: Option<ScreenParts>,
    /// The fallback idle source; `None` (and no XI2) is manual mode.
    pub idle: Option<Arc<dyn IdleSource>>,
}

/// What [`Tracker::apply_sensor_policy`] last did.
#[derive(Default)]
struct Applied {
    kinds: InputKinds,
    input_error: Option<(Instant, String)>,
    usage_error: Option<(Instant, String)>,
}

/// The sensors of one session, consent-gated.
pub struct Tracker {
    kind: SessionKind,
    desktop: Option<String>,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
    device: DeviceInfo,
    driver: Arc<dyn SensorDriver>,
    gated: Arc<ConsentGatedTracker>,
    input: Arc<InputLog>,
    usage: Arc<UsageRecorder>,
    screen: Option<ScreenParts>,
    idle: Option<Arc<dyn IdleSource>>,
    sink: Mutex<Option<UnboundedSender<CaptureEvent>>>,
    capture_busy: Arc<AtomicBool>,
    applied: Mutex<Applied>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Tracker {
    /// A tracker of `parts`. Nothing is started: the sensors run only once
    /// [`Tracker::apply_sensor_policy`] says the session tracks.
    pub fn new(parts: TrackerParts) -> Self {
        let gated = Arc::new(ConsentGatedTracker::new(
            parts.active_window,
            parts.running_apps,
            parts.lifecycle.clone(),
        ));
        Self {
            kind: parts.kind,
            desktop: parts.desktop,
            lifecycle: parts.lifecycle,
            device: parts.device,
            driver: parts.driver,
            gated,
            input: Arc::new(InputLog::new()),
            usage: Arc::new(UsageRecorder::new()),
            screen: parts.screen,
            idle: parts.idle,
            sink: Mutex::new(None),
            capture_busy: Arc::new(AtomicBool::new(false)),
            applied: Mutex::new(Applied::default()),
        }
    }

    /// The native adapters of the probed session: X11 (display from the
    /// probe), Wayland (portal on `session_bus_address`, idle from `buses`),
    /// or none. Nothing connects until it is used.
    pub fn for_session(
        info: &SessionInfo,
        buses: &IdleBuses,
        session_bus_address: Option<&str>,
        lifecycle: Arc<Mutex<TrackerLifecycle>>,
        device: DeviceInfo,
    ) -> Self {
        let none = |kind: SessionKind, lifecycle, device| TrackerParts {
            kind,
            desktop: info.desktop.clone(),
            lifecycle,
            device,
            driver: Arc::new(NullSensorDriver),
            active_window: Arc::new(WaylandActiveWindowProvider::new()),
            running_apps: Arc::new(ProcFsRunningAppsProvider::for_current_user(
                ProgramFilter::AppScopes,
            )),
            screen: None,
            idle: None,
        };
        let parts = match (info.kind, info.display.as_deref()) {
            (SessionKind::X11, Some(display)) => {
                let window = Arc::new(X11ActiveWindowProvider::with_display(
                    display,
                    Some(Arc::new(SysProcFsReader::new())),
                ));
                TrackerParts {
                    kind: SessionKind::X11,
                    desktop: info.desktop.clone(),
                    lifecycle,
                    device,
                    driver: Arc::new(NativeX11SensorDriver::new(display)),
                    active_window: window.clone(),
                    running_apps: Arc::new(ProcFsRunningAppsProvider::for_current_user(
                        ProgramFilter::WindowOwners(window),
                    )),
                    screen: Some(ScreenParts {
                        source: Arc::new(X11ScreenSource::with_display(display)),
                        backend: "x11",
                        at_block_end: true,
                        reset: None,
                    }),
                    idle: Some(Arc::new(X11ScreenSaverIdleSource::with_display(display))),
                }
            }
            (SessionKind::Wayland, _) => {
                let mut parts = none(SessionKind::Wayland, lifecycle, device);
                parts.screen = session_bus_address.map(|address| {
                    let portal = Arc::new(PortalScreenSource::new(address));
                    let reset = portal.clone();
                    ScreenParts {
                        source: portal,
                        backend: "portal",
                        at_block_end: false,
                        reset: Some(Arc::new(move || reset.reset())),
                    }
                });
                parts.idle = probe_idle_sources(info.desktop.as_deref(), buses).map(Arc::from);
                parts
            }
            (kind, _) => none(
                if kind.is_graphical() {
                    SessionKind::None
                } else {
                    kind
                },
                lifecycle,
                device,
            ),
        };
        Self::new(parts)
    }

    /// The per-second input log (the live view reads it).
    pub fn input(&self) -> Arc<InputLog> {
        self.input.clone()
    }

    /// Foreground usage (the clients' "top apps").
    pub fn usage(&self) -> Arc<UsageRecorder> {
        self.usage.clone()
    }

    pub fn driver(&self) -> Arc<dyn SensorDriver> {
        self.driver.clone()
    }

    /// Is input being measured right now (the listener runs and delivers)?
    fn input_measuring(&self) -> bool {
        self.driver.is_input_running() && self.input.is_healthy()
    }

    fn sensor_retry_due(error: &Option<(Instant, String)>) -> bool {
        error
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() >= SENSOR_RETRY)
    }
}

impl Machine for Tracker {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn awake_millis(&self) -> u64 {
        crate::agent::power::awake_millis()
    }

    /// The XI2 view (the user's own input, never XTEST) while it measures;
    /// otherwise the session's idle source; in manual mode always now.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.input_measuring() {
            return self.input.last_physical_input();
        }
        match &self.idle {
            Some(idle) => match idle.last_input(now) {
                Ok(last) => last,
                Err(e) => {
                    tracing::debug!(source = idle.kind(), error = %e, "the idle source did not answer");
                    None
                }
            },
            None => Some(now),
        }
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.input_measuring() {
            return self
                .input
                .measurements(since + TimeDelta::seconds(1), now)
                .active_seconds
                .into_iter()
                .min()
                .and_then(|second| DateTime::from_timestamp(second, 0));
        }
        // Manual mode: nothing tells that the user is back.
        self.idle.as_ref()?;
        self.last_input(now).filter(|last| *last > since)
    }

    fn set_capture_sink(&self, sink: UnboundedSender<CaptureEvent>) {
        *lock(&self.sink) = Some(sink);
    }

    fn request_capture(&self, block: BlockId, blur: bool) -> bool {
        let Some(screen) = self.screen.clone() else {
            return false;
        };
        let Some(sink) = lock(&self.sink).clone() else {
            return false;
        };
        if self.capture_busy.swap(true, Ordering::SeqCst) {
            tracing::debug!("a screenshot is still being taken; this one is skipped");
            return false;
        }
        let (ticket, blur) = {
            let lifecycle = lock(&self.lifecycle);
            match lifecycle.acquire_ticket(AcquisitionCapability::ScreenCapture) {
                Ok(ticket) => (
                    ticket,
                    blur || lifecycle.effective_permissions().blur_screenshots,
                ),
                Err(e) => {
                    tracing::debug!(error = %e, "no screenshot admitted");
                    self.capture_busy.store(false, Ordering::SeqCst);
                    return false;
                }
            }
        };
        let lifecycle = self.lifecycle.clone();
        let busy = self.capture_busy.clone();
        let spawned = std::thread::Builder::new()
            .name("dosi-capture".into())
            .spawn(move || {
                let capture = screen
                    .source
                    .capture(CaptureTarget::ActiveMonitor)
                    .and_then(|frame| encode_frame(frame, blur));
                let capture = match capture {
                    Ok(capture) => {
                        // Consent or policy changed while it ran: discarded.
                        let valid = lock(&lifecycle).validate_ticket(&ticket);
                        match valid {
                            Ok(()) => Some(ProvenanceScreenCapture {
                                capture,
                                generation: ticket.generation,
                            }),
                            Err(e) => {
                                tracing::debug!(error = %e, "a screenshot finished after the policy changed; discarded");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "the screenshot failed");
                        None
                    }
                };
                busy.store(false, Ordering::SeqCst);
                let _ = sink.send(CaptureEvent {
                    block_id: block,
                    capture,
                });
            });
        match spawned {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(?e, "could not start the capture thread");
                self.capture_busy.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        let screen = self.screen.clone().filter(|s| s.at_block_end)?;
        let (ticket, blur) = {
            let lifecycle = lock(&self.lifecycle);
            let ticket = lifecycle
                .acquire_ticket(AcquisitionCapability::ScreenCapture)
                .ok()?;
            (
                ticket,
                blur || lifecycle.effective_permissions().blur_screenshots,
            )
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("dosi-capture".into())
            .spawn(move || {
                let result = screen
                    .source
                    .capture(CaptureTarget::ActiveMonitor)
                    .and_then(|frame| encode_frame(frame, blur));
                let _ = tx.send(result);
            })
            .ok()?;
        let capture = match rx.recv_timeout(CAPTURE_AT_END_TIMEOUT) {
            Ok(Ok(capture)) => capture,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "the screenshot at the block's end failed");
                return None;
            }
            Err(_) => {
                tracing::warn!("the screenshot at the block's end took too long; skipped");
                return None;
            }
        };
        lock(&self.lifecycle).validate_ticket(&ticket).ok()?;
        Some(ProvenanceScreenCapture {
            capture,
            generation: ticket.generation,
        })
    }

    fn snapshot(
        &self,
        block: &BlockSpec<'_>,
        screen: Option<ProvenanceScreenCapture>,
    ) -> BuiltBlock {
        let perms = block.perms;
        let (started_at, ended_at) = (block.started_at, block.ended_at);
        // Acquired now, through the consent gate: nothing when the
        // capability is not admitted (time-only, revoked, paused).
        let window_now = if perms.active_window {
            self.gated.current_active_window().ok().flatten()
        } else {
            None
        };
        if let Some(window) = window_now.clone() {
            self.usage.record(ended_at.timestamp_millis(), Some(window));
        }
        let running_programs = if perms.running_programs {
            self.gated.running_programs().unwrap_or_else(|e| {
                tracing::debug!(error = %e, "no running programs for this block");
                Vec::new()
            })
        } else {
            Vec::new()
        };
        let input = if perms.keyboard || perms.mouse {
            let mut input = self.input.measurements(started_at, ended_at);
            let from = (started_at + INPUT_START_GRACE).min(ended_at);
            input.healthy = self.input.healthy_between(from, ended_at);
            input
        } else {
            Default::default()
        };
        // A capture taken in an earlier generation (consent or policy
        // changed since) is never attached.
        let screen = screen.filter(|_| perms.screenshot).filter(|shot| {
            let lifecycle = lock(&self.lifecycle);
            lifecycle.state() == TrackerState::Tracking
                && lifecycle.generation() == shot.generation
                && lifecycle.effective_permissions().screenshot
        });
        let measured = Measurements {
            key_presses: perms.keyboard.then_some(input.key_presses_ms),
            clicks: perms.mouse.then_some(input.clicks_ms),
            active_seconds: input.active_seconds,
            injected: input.injected,
            input_available: input.healthy,
            slices: if perms.active_window {
                self.usage
                    .slices_between(started_at.timestamp_millis(), ended_at.timestamp_millis())
            } else {
                Vec::new()
            },
            window_now,
            running_programs,
            screen: screen.map(|shot| shot.capture),
        };
        let zone = Zone {
            time_zone: self.device.time_zone.clone(),
            utc_offset_minutes: self.device.utc_offset_minutes(started_at),
        };
        build_activity(block, measured, &self.device, zone)
    }

    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        let max = TimeDelta::from_std(interval).unwrap_or(TimeDelta::minutes(10));
        kept_idle_blocks(
            project_id,
            from,
            to,
            max,
            description,
            &self.device,
            |start| Zone {
                time_zone: self.device.time_zone.clone(),
                utc_offset_minutes: self.device.utc_offset_minutes(start),
            },
        )
    }

    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn current_app(&self) -> Option<String> {
        if self.kind != SessionKind::X11 {
            return None;
        }
        self.gated
            .current_active_window()
            .ok()
            .flatten()
            .map(|window| window.app_name)
            .filter(|name| !name.trim().is_empty())
    }

    fn random_fraction(&self) -> f64 {
        crate::agent::blocks::random_fraction()
    }

    /// Configure the lifecycle with the effective permissions and start or
    /// stop it and the sensors. Synchronous: a withdrawn sensor's thread is
    /// gone when this returns. Repeating the same policy changes nothing.
    fn apply_sensor_policy(&self, active: bool, perms: Option<&ConsentedPermissions>) {
        let active = active && perms.is_some();
        let effective = match perms {
            Some(perms) if active => *perms,
            _ => ConsentedPermissions::all_disabled(),
        };
        let mut applied = lock(&self.applied);

        // 1. Admission first: every change bumps the generation, so samples
        //    in flight under the old policy are discarded.
        {
            let mut lifecycle = lock(&self.lifecycle);
            lifecycle.configure(effective);
            if active && effective.has_any_active_capability() {
                if lifecycle.state() != TrackerState::Tracking {
                    if let Err(e) = lifecycle.start() {
                        tracing::debug!(error = %e, "the acquisition lifecycle stays stopped");
                    }
                }
            } else if matches!(
                lifecycle.state(),
                TrackerState::Tracking | TrackerState::Paused(_) | TrackerState::Locked
            ) {
                let _ = lifecycle.stop();
            }
        }

        // 2. Input counting (XI2), only the effective kinds.
        let kinds = InputKinds {
            keyboard: active && effective.keyboard,
            mouse: active && effective.mouse,
        };
        if kinds.any() {
            let changed = kinds != applied.kinds;
            if (changed || !self.driver.is_input_running())
                && (changed || Self::sensor_retry_due(&applied.input_error))
            {
                match self.driver.start_input(self.input.clone(), kinds) {
                    Ok(()) => applied.input_error = None,
                    Err(e) => {
                        tracing::warn!(error = %e, "input counting could not start");
                        applied.input_error = Some((Instant::now(), e.to_string()));
                    }
                }
            }
        } else {
            self.driver.stop_input();
            applied.input_error = None;
        }
        applied.kinds = kinds;

        // 3. Foreground usage, only with the active window effective.
        let want_usage = active && effective.active_window;
        self.usage.set_active(want_usage);
        if want_usage {
            if !self.driver.is_usage_running() && Self::sensor_retry_due(&applied.usage_error) {
                let provider: Arc<dyn ActiveWindowProvider> = self.gated.clone();
                match self.driver.start_usage(self.usage.clone(), provider) {
                    Ok(()) => applied.usage_error = None,
                    Err(e) => {
                        tracing::warn!(error = %e, "the usage sampler could not start");
                        applied.usage_error = Some((Instant::now(), e.to_string()));
                    }
                }
            }
        } else {
            self.driver.stop_usage();
            applied.usage_error = None;
        }
    }

    fn session_summary(&self) -> SessionSummary {
        let idle_source = if self.input_measuring() {
            Some("xi2".to_owned())
        } else {
            self.idle.as_ref().map(|idle| idle.kind().to_owned())
        };
        SessionSummary {
            kind: self.kind,
            desktop: self.desktop.clone(),
            idle_source: idle_source.filter(|_| self.kind.is_graphical()),
        }
    }

    fn platform(&self) -> PlatformCapabilities {
        let screenshot = match &self.screen {
            Some(screen) => PlatformCapability {
                availability: platform_availability(screen.source.availability()),
                backend: Some(screen.backend.to_owned()),
            },
            None => PlatformCapability::unsupported(NO_PORTAL),
        };
        match self.kind {
            SessionKind::X11 => {
                let applied = lock(&self.applied);
                let sensor = |error: &Option<(Instant, String)>, backend: &str| match error {
                    Some((_, reason)) => PlatformCapability {
                        availability: PlatformAvailability::Unavailable(reason.clone()),
                        backend: Some(backend.to_owned()),
                    },
                    None => PlatformCapability::available(backend),
                };
                PlatformCapabilities {
                    keyboard: sensor(&applied.input_error, "xi2"),
                    mouse: sensor(&applied.input_error, "xi2"),
                    active_window: sensor(&applied.usage_error, "x11"),
                    running_programs: PlatformCapability::available("procfs"),
                    screenshot,
                }
            }
            SessionKind::Wayland => PlatformCapabilities {
                keyboard: PlatformCapability::unsupported(WAYLAND_UNSUPPORTED),
                mouse: PlatformCapability::unsupported(WAYLAND_UNSUPPORTED),
                active_window: PlatformCapability::unsupported(WAYLAND_UNSUPPORTED),
                running_programs: PlatformCapability::available("procfs"),
                screenshot,
            },
            SessionKind::Tty | SessionKind::None => PlatformCapabilities::nothing(NO_SESSION),
        }
    }

    fn retry_capture_permission(&self) {
        if let Some(reset) = self.screen.as_ref().and_then(|s| s.reset.clone()) {
            reset();
        }
    }
}

fn platform_availability(availability: SourceAvailability) -> PlatformAvailability {
    match availability {
        SourceAvailability::Available => PlatformAvailability::Available,
        SourceAvailability::Unsupported(reason) => PlatformAvailability::Unsupported(reason),
        SourceAvailability::Denied => PlatformAvailability::Denied,
        SourceAvailability::NeedsApproval => PlatformAvailability::NeedsApproval,
        SourceAvailability::Unavailable(reason) => PlatformAvailability::Unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::EndReason;
    use crate::consent::{ConsentManager, LocalConsent};
    use crate::tracking::capture::RawFrame;
    use crate::tracking::fake::FakeAcquisitionProvider;
    use crate::tracking::model::WindowInfo;
    use image::{Rgb, RgbImage};

    const PROJECT: &str = "3f2a9c1e-6b7d-4e8f-9a0b-1c2d3e4f5a6b";

    fn all() -> ConsentedPermissions {
        ConsentedPermissions {
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            blur_screenshots: false,
        }
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
            device_name: Some("work-laptop".into()),
            os_version: "Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)".into(),
            agent_version: "9.9.9".into(),
            platform: "linux".into(),
            time_zone: Some("UTC".into()),
        }
    }

    /// A screen whose captures the test can hold until it releases them.
    #[derive(Default)]
    struct FakeScreen {
        calls: AtomicU32,
        hold: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        availability: Mutex<Option<SourceAvailability>>,
    }

    impl FakeScreen {
        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }

        /// Hold the next capture: `entered` fires inside it, `release` ends it.
        fn hold(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            *lock(&self.entered) = Some(entered_tx);
            *lock(&self.hold) = Some(release_rx);
            (entered_rx, release_tx)
        }
    }

    impl ScreenSource for FakeScreen {
        fn capture(&self, _target: CaptureTarget) -> Result<RawFrame, TrackingError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = lock(&self.entered).take() {
                let _ = entered.send(());
            }
            if let Some(release) = lock(&self.hold).take() {
                let _ = release.recv();
            }
            Ok(RawFrame {
                image: RgbImage::from_pixel(64, 48, Rgb([10, 20, 30])),
                captured_at: Utc::now(),
                monitor: None,
            })
        }

        fn availability(&self) -> SourceAvailability {
            lock(&self.availability)
                .clone()
                .unwrap_or(SourceAvailability::Available)
        }
    }

    struct FixedIdle(Option<DateTime<Utc>>);

    impl IdleSource for FixedIdle {
        fn last_input(&self, _now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError> {
            Ok(self.0)
        }

        fn kind(&self) -> &'static str {
            "screensaver"
        }
    }

    struct Fixture {
        tracker: Tracker,
        driver: Arc<FakeSensorDriver>,
        acquisition: Arc<FakeAcquisitionProvider>,
        screen: Arc<FakeScreen>,
        lifecycle: Arc<Mutex<TrackerLifecycle>>,
    }

    fn fixture_with(
        kind: SessionKind,
        consent: LocalConsent,
        idle: Option<Arc<dyn IdleSource>>,
        at_end: bool,
    ) -> Fixture {
        let lifecycle = Arc::new(Mutex::new(TrackerLifecycle::new(ConsentManager::new(
            consent, None,
        ))));
        let driver = Arc::new(FakeSensorDriver::new());
        let acquisition = Arc::new(FakeAcquisitionProvider::new());
        let screen = Arc::new(FakeScreen::default());
        let tracker = Tracker::new(TrackerParts {
            kind,
            desktop: Some("GNOME".into()),
            lifecycle: lifecycle.clone(),
            device: device(),
            driver: driver.clone(),
            active_window: acquisition.clone(),
            running_apps: acquisition.clone(),
            screen: Some(ScreenParts {
                source: screen.clone(),
                backend: if at_end { "x11" } else { "portal" },
                at_block_end: at_end,
                reset: None,
            }),
            idle,
        });
        Fixture {
            tracker,
            driver,
            acquisition,
            screen,
            lifecycle,
        }
    }

    fn x11() -> Fixture {
        fixture_with(
            SessionKind::X11,
            LocalConsent::Granted(all()),
            Some(Arc::new(FixedIdle(None))),
            true,
        )
    }

    fn spec(perms: &ConsentedPermissions, from: DateTime<Utc>, to: DateTime<Utc>) -> BlockSpec<'_> {
        BlockSpec {
            project_id: PROJECT,
            perms,
            started_at: from,
            ended_at: to,
            end_reason: Some(EndReason::Interval),
            description: None,
        }
    }

    fn sink(tracker: &Tracker) -> tokio::sync::mpsc::UnboundedReceiver<CaptureEvent> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tracker.set_capture_sink(tx);
        rx
    }

    fn next_capture(rx: &mut tokio::sync::mpsc::UnboundedReceiver<CaptureEvent>) -> CaptureEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(event) = rx.try_recv() {
                return event;
            }
            assert!(Instant::now() < deadline, "no capture event");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn nothing_runs_until_the_session_tracks_and_everything_stops_after() {
        let f = x11();
        assert!(!f.driver.is_input_running() && !f.driver.is_usage_running());
        assert_eq!(lock(&f.lifecycle).state(), TrackerState::Unconfigured);

        f.tracker.apply_sensor_policy(true, Some(&all()));
        assert!(f.driver.is_input_running() && f.driver.is_usage_running());
        assert_eq!(lock(&f.lifecycle).state(), TrackerState::Tracking);
        assert!(f.driver.last_keyboard_allowed() && f.driver.last_mouse_allowed());

        f.tracker.apply_sensor_policy(false, None);
        assert!(!f.driver.is_input_running() && !f.driver.is_usage_running());
        assert_eq!(lock(&f.lifecycle).state(), TrackerState::Stopped);
        assert_eq!(
            (
                f.driver.input_stopped_count(),
                f.driver.usage_stopped_count()
            ),
            (1, 1)
        );
        assert!(
            !f.tracker.input.is_healthy(),
            "input is not measured once stopped"
        );
    }

    #[test]
    fn only_the_effective_sensors_run() {
        let f = x11();
        let keyboard_only = ConsentedPermissions {
            keyboard: true,
            ..ConsentedPermissions::all_disabled()
        };
        f.tracker.apply_sensor_policy(true, Some(&keyboard_only));
        assert!(f.driver.is_input_running());
        assert!(f.driver.last_keyboard_allowed() && !f.driver.last_mouse_allowed());
        assert!(
            !f.driver.is_usage_running(),
            "the active window is not effective"
        );
        assert!(!f.tracker.usage.is_active());
    }

    #[test]
    fn time_only_tracking_keeps_the_lifecycle_stopped_and_still_builds_blocks() {
        let f = fixture_with(
            SessionKind::X11,
            LocalConsent::Granted(ConsentedPermissions::all_disabled()),
            Some(Arc::new(FixedIdle(None))),
            true,
        );
        let none = ConsentedPermissions::all_disabled();
        f.tracker.apply_sensor_policy(true, Some(&none));
        assert_eq!(lock(&f.lifecycle).state(), TrackerState::Stopped);
        assert!(!f.driver.is_input_running() && !f.driver.is_usage_running());
        let now = Utc::now();
        let built = f
            .tracker
            .snapshot(&spec(&none, now - TimeDelta::minutes(10), now), None);
        assert_eq!(built.activity.project_id, PROJECT);
        assert_eq!(
            (built.activity.keyboard_hits, built.activity.mouse_clicks),
            (None, None)
        );
        assert!(built.activity.running_programs.is_empty() && built.screen.is_none());
        assert_eq!(
            f.acquisition.process_calls() + f.acquisition.window_calls(),
            0
        );
    }

    #[test]
    fn repeating_the_same_policy_restarts_nothing_and_keeps_the_generation() {
        let f = x11();
        f.tracker.apply_sensor_policy(true, Some(&all()));
        let generation = lock(&f.lifecycle).generation();
        for _ in 0..5 {
            f.tracker.apply_sensor_policy(true, Some(&all()));
        }
        assert_eq!(lock(&f.lifecycle).generation(), generation);
        assert_eq!(
            (
                f.driver.input_started_count(),
                f.driver.usage_started_count()
            ),
            (1, 1)
        );
    }

    #[test]
    fn a_failing_listener_is_retried_slowly_and_reported_unavailable() {
        let f = x11();
        f.driver.set_fail_input(true);
        f.tracker.apply_sensor_policy(true, Some(&all()));
        f.tracker.apply_sensor_policy(true, Some(&all()));
        assert_eq!(f.driver.input_attempts(), 1, "not retried on every publish");
        let platform = f.tracker.platform();
        assert!(matches!(
            platform.keyboard.availability,
            PlatformAvailability::Unavailable(_)
        ));
        assert!(
            platform.keyboard.usable(),
            "still effective: its counts are simply unmeasured"
        );
        let now = Utc::now();
        let built = f
            .tracker
            .snapshot(&spec(&all(), now - TimeDelta::minutes(10), now), None);
        assert_eq!(built.activity.keyboard_hits, None, "not measured, never 0");
        assert!(built.activity.timeline.is_empty());
    }

    #[test]
    fn measured_input_is_counted_and_injected_input_is_not() {
        let f = x11();
        f.tracker.apply_sensor_policy(true, Some(&all()));
        let start = Utc::now() - TimeDelta::seconds(1);
        let log = f.tracker.input();
        let second = start.timestamp() + 10;
        log.record_key(second, false);
        log.record_key(second, false);
        log.record_click(second + 1, false);
        log.record_key(second + 2, true);
        let end = start + TimeDelta::minutes(2);
        let built = f.tracker.snapshot(&spec(&all(), start, end), None);
        assert_eq!(built.activity.keyboard_hits, Some(2));
        assert_eq!(built.activity.mouse_clicks, Some(1));
        assert_eq!(built.activity.injected_input_events, Some(1));
        assert!(built.activity.productivity.is_some());
        assert_eq!(
            built.activity.running_programs.len(),
            2,
            "acquired at the block's end"
        );
    }

    #[test]
    fn a_wayland_session_snapshot_omits_every_unmeasured_field() {
        let f = fixture_with(
            SessionKind::Wayland,
            LocalConsent::Granted(all()),
            Some(Arc::new(FixedIdle(None))),
            false,
        );
        // What the session rules make effective on Wayland.
        let wayland = ConsentedPermissions {
            keyboard: false,
            mouse: false,
            active_window: false,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&wayland));
        assert!(!f.driver.is_input_running() && !f.driver.is_usage_running());
        let now = Utc::now();
        let built = f
            .tracker
            .snapshot(&spec(&wayland, now - TimeDelta::minutes(10), now), None);
        let json = serde_json::to_value(&built.activity).unwrap();
        for absent in [
            "productivity",
            "idleSeconds",
            "timeline",
            "injectedInputEvents",
        ] {
            assert!(json.get(absent).is_none(), "{absent}: {json}");
        }
        assert!(json["keyboardHits"].is_null() && json["mouseClicks"].is_null());
        assert_eq!(json["activeWindows"], serde_json::json!([]));
        assert_eq!(json["runningPrograms"].as_array().unwrap().len(), 2);
        assert_eq!(f.tracker.current_app(), None, "no current app on Wayland");
        let platform = f.tracker.platform();
        assert!(!platform.keyboard.usable() && !platform.active_window.usable());
        assert!(platform.screenshot.usable() && platform.running_programs.usable());
    }

    #[test]
    fn a_screenshot_is_taken_on_the_capture_thread_and_validated_after() {
        let f = x11();
        let mut events = sink(&f.tracker);
        // Not tracking: nothing is admitted, nothing is captured.
        assert!(!f.tracker.request_capture(1, false));
        assert_eq!(f.screen.calls(), 0);

        f.tracker.apply_sensor_policy(true, Some(&all()));
        assert!(f.tracker.request_capture(1, false));
        let event = next_capture(&mut events);
        assert_eq!(event.block_id, 1);
        let shot = event.capture.expect("a capture");
        assert!(!shot.capture.jpeg.is_empty() && shot.capture.thumb_jpeg.is_some());
        assert!(!shot.capture.blurred);
        assert_eq!(shot.generation, lock(&f.lifecycle).generation());

        // Blur required by the policy: the capture is blurred.
        let blur = ConsentedPermissions {
            blur_screenshots: true,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&blur));
        assert!(f.tracker.request_capture(2, false));
        assert!(next_capture(&mut events).capture.unwrap().capture.blurred);
    }

    #[test]
    fn an_in_flight_screenshot_is_discarded_when_consent_is_revoked_before_it_returns() {
        let f = x11();
        let mut events = sink(&f.tracker);
        f.tracker.apply_sensor_policy(true, Some(&all()));
        let (entered, release) = f.screen.hold();
        assert!(f.tracker.request_capture(7, false));
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        // Revoked while the screen is being read.
        lock(&f.lifecycle)
            .update_consent(LocalConsent::Revoked)
            .unwrap();
        f.tracker.apply_sensor_policy(false, None);
        release.send(()).unwrap();
        let event = next_capture(&mut events);
        assert_eq!(event.block_id, 7);
        assert!(
            event.capture.is_none(),
            "the sample of a revoked consent is discarded"
        );
    }

    #[test]
    fn a_capture_of_an_earlier_generation_is_never_attached() {
        let f = x11();
        f.tracker.apply_sensor_policy(true, Some(&all()));
        let shot = f
            .tracker
            .capture_screen(false)
            .expect("X11 captures at the end");
        // The policy changed (keyboard withdrawn): a new generation.
        let narrower = ConsentedPermissions {
            keyboard: false,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&narrower));
        let now = Utc::now();
        let built = f.tracker.snapshot(
            &spec(&narrower, now - TimeDelta::minutes(10), now),
            Some(shot),
        );
        assert!(built.screen.is_none());
        let fresh = f.tracker.capture_screen(false).unwrap();
        let built = f.tracker.snapshot(
            &spec(&narrower, now - TimeDelta::minutes(10), now),
            Some(fresh),
        );
        assert!(
            built.screen.is_some(),
            "a capture of the current generation is kept"
        );
    }

    #[test]
    fn the_portal_is_never_asked_at_a_block_end_and_one_capture_runs_at_a_time() {
        let f = fixture_with(
            SessionKind::Wayland,
            LocalConsent::Granted(all()),
            Some(Arc::new(FixedIdle(None))),
            false,
        );
        let mut events = sink(&f.tracker);
        let wayland = ConsentedPermissions {
            keyboard: false,
            mouse: false,
            active_window: false,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&wayland));
        assert!(f.tracker.capture_screen(false).is_none());
        assert_eq!(f.screen.calls(), 0);
        let (entered, release) = f.screen.hold();
        assert!(f.tracker.request_capture(1, false));
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            !f.tracker.request_capture(2, false),
            "the dialog may still be open"
        );
        release.send(()).unwrap();
        assert!(next_capture(&mut events).capture.is_some());
    }

    #[test]
    fn a_refused_portal_narrows_what_may_be_captured() {
        let f = fixture_with(
            SessionKind::Wayland,
            LocalConsent::Granted(all()),
            Some(Arc::new(FixedIdle(None))),
            false,
        );
        *lock(&f.screen.availability) = Some(SourceAvailability::Denied);
        let platform = f.tracker.platform();
        assert_eq!(
            platform.screenshot.availability,
            PlatformAvailability::Denied
        );
        assert!(!platform.availability().screenshot);
        *lock(&f.screen.availability) = Some(SourceAvailability::NeedsApproval);
        assert!(
            f.tracker.platform().availability().screenshot,
            "waiting for approval: asked again at the next moment"
        );
    }

    #[test]
    fn idle_comes_from_xi2_then_the_idle_source_and_manual_mode_counts_as_present() {
        let earlier = Utc::now() - TimeDelta::minutes(3);
        let f = fixture_with(
            SessionKind::X11,
            LocalConsent::Granted(all()),
            Some(Arc::new(FixedIdle(Some(earlier)))),
            true,
        );
        let now = Utc::now();
        // Not measuring input: the screensaver's answer.
        assert_eq!(f.tracker.last_input(now), Some(earlier));
        assert_eq!(
            f.tracker.session_summary().idle_source.as_deref(),
            Some("screensaver")
        );
        // Measuring: the listener's physical input only.
        f.tracker.apply_sensor_policy(true, Some(&all()));
        f.tracker.input().record_key(now.timestamp() - 5, false);
        assert_eq!(
            f.tracker.last_input(now),
            DateTime::from_timestamp(now.timestamp() - 5, 0)
        );
        assert_eq!(
            f.tracker.session_summary().idle_source.as_deref(),
            Some("xi2")
        );
        assert_eq!(
            f.tracker
                .first_input_after(now - TimeDelta::minutes(1), now),
            DateTime::from_timestamp(now.timestamp() - 5, 0)
        );

        // No idle source on Wayland: manual mode.
        let manual = fixture_with(
            SessionKind::Wayland,
            LocalConsent::Granted(all()),
            None,
            false,
        );
        assert_eq!(manual.tracker.last_input(now), Some(now), "always present");
        assert_eq!(
            manual
                .tracker
                .first_input_after(now - TimeDelta::minutes(1), now),
            None
        );
        assert_eq!(manual.tracker.session_summary().idle_source, None);
    }

    #[test]
    fn the_current_app_is_known_on_x11_with_the_active_window_effective_only() {
        let f = x11();
        assert_eq!(f.tracker.current_app(), None, "not tracking");
        f.tracker.apply_sensor_policy(true, Some(&all()));
        assert_eq!(f.tracker.current_app().as_deref(), Some("code"));
        let no_window = ConsentedPermissions {
            active_window: false,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&no_window));
        assert_eq!(f.tracker.current_app(), None);
    }

    #[test]
    fn running_programs_need_their_own_admission() {
        let f = x11();
        let no_programs = ConsentedPermissions {
            running_programs: false,
            ..all()
        };
        f.tracker.apply_sensor_policy(true, Some(&no_programs));
        let now = Utc::now();
        // A stale spec still asking for them: the gate refuses.
        let built = f
            .tracker
            .snapshot(&spec(&all(), now - TimeDelta::minutes(10), now), None);
        assert!(built.activity.running_programs.is_empty());
        assert_eq!(f.acquisition.process_calls(), 0);
        f.acquisition.set_canned_window(Some(WindowInfo {
            app_name: "firefox".into(),
            window_title: "Docs".into(),
            ..Default::default()
        }));
        assert_eq!(f.tracker.current_app().as_deref(), Some("firefox"));
    }

    #[test]
    fn no_session_offers_nothing() {
        let f = fixture_with(SessionKind::Tty, LocalConsent::Granted(all()), None, false);
        let platform = f.tracker.platform();
        assert!(!platform.availability().keyboard && !platform.availability().screenshot);
        assert_eq!(f.tracker.session_summary().idle_source, None);
        assert!(!f.tracker.session_summary().kind.is_graphical());
    }

    #[test]
    fn kept_idle_blocks_carry_the_device_context() {
        let f = x11();
        let now = Utc::now();
        let blocks = f.tracker.kept_idle_blocks(
            PROJECT,
            now - TimeDelta::minutes(25),
            now,
            Duration::from_secs(600),
            Some("Call"),
        );
        assert_eq!(blocks.len(), 3);
        assert!(blocks.iter().all(|b| b.platform.as_deref() == Some("linux")
            && b.time_zone.as_deref() == Some("UTC")
            && b.keyboard_hits.is_none()));
    }
}
