//! The worker's tests: its helpers, and the worker itself driven through a
//! fake machine ([`FakeMachine`]) and the [`Harness`] around it.

mod calendar;
mod consent;
mod e2e;
mod helpers;
mod idle_without_hooks;
mod lifecycle;
mod recording;
mod sensors;
mod sign_in;
mod terminal_errors;
mod workspaces;

use std::sync::Mutex;

use chrono::{FixedOffset, Local};

use super::account::{friendly_login_error, is_auth_rejection};
use super::control::{ControlEvent, Farewell};
use super::queue::push_recent_block;
use super::session::session_for;
use super::uploader::UploadEvent;
use super::workspaces::ONE_WORKSPACE;
use super::*;
use crate::api::{ApiClient, ApiError, AuthError, TokenCell};
use crate::config::{CapturePermissions, ConsentedPermissions, LocalConsent, MemoryConsentStore};
use crate::credentials::ProjectCache;
use crate::model::{Activity, Heartbeat};
use crate::session_events::SystemEvent;
use crate::state::{RecentBlock, UploadState, RECENT_BLOCKS};
use crate::storage;
use crate::tracking::screenshot::ScreenCapture;
use crate::tracking::system::DeviceInfo;
use crate::tracking::{build_activity, BlockSpec, Measurements, Zone};

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// 5 October 2026 at `hms`, UTC.
fn at(hms: &str) -> DateTime<Utc> {
    utc(&format!("2026-10-05T{hms}Z"))
}

// ---- The worker, driven through a fake machine -----------------------------------------

/// A machine whose clocks, input and screen the test controls.
///
/// Input is modelled as a real machine sees it: the user's own input
/// (`inputs`) reaches the input hooks only while they are attached and only
/// from the moment they were; Windows' idle clock sees all of it, plus
/// software-injected input (`injected`, a mouse jiggler).
struct FakeMachine {
    now: Mutex<DateTime<Utc>>,
    awake_ms: Mutex<u64>,
    inputs: Mutex<Vec<DateTime<Utc>>>,
    injected: Mutex<Vec<DateTime<Utc>>>,
    /// Since when the input hooks are attached (`None`: they are not).
    hooks_since: Mutex<Option<DateTime<Utc>>>,
    captures: Mutex<u32>,
    fraction: Mutex<f64>,
    device: DeviceInfo,
    tracking_active: std::sync::atomic::AtomicBool,
    input_running: std::sync::atomic::AtomicBool,
    usage_running: std::sync::atomic::AtomicBool,
    input_starts: std::sync::atomic::AtomicUsize,
    input_stops: std::sync::atomic::AtomicUsize,
    usage_starts: std::sync::atomic::AtomicUsize,
    usage_stops: std::sync::atomic::AtomicUsize,
    last_keyboard: std::sync::atomic::AtomicBool,
    last_mouse: std::sync::atomic::AtomicBool,
    last_screenshot: std::sync::atomic::AtomicBool,
    last_webcam: std::sync::atomic::AtomicBool,
    last_perms: Mutex<Option<CapturePermissions>>,
    webcam_captures: std::sync::atomic::AtomicUsize,
    window_queries: std::sync::atomic::AtomicUsize,
    process_queries: std::sync::atomic::AtomicUsize,
    snapshot_calls: std::sync::atomic::AtomicUsize,
    on_snapshot: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl FakeMachine {
    fn new(start: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(start),
            awake_ms: Mutex::new(1_000_000),
            inputs: Mutex::new(Vec::new()),
            injected: Mutex::new(Vec::new()),
            hooks_since: Mutex::new(None),
            captures: Mutex::new(0),
            fraction: Mutex::new(0.5),
            device: DeviceInfo {
                device_id: Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into()),
                device_name: Some("DESKTOP-1".into()),
                os_version: Some("Windows 11 Pro 23H2 (10.0.22631)".into()),
                platform: "windows",
                agent_version: "9.9.9",
            },
            tracking_active: std::sync::atomic::AtomicBool::new(false),
            input_running: std::sync::atomic::AtomicBool::new(false),
            usage_running: std::sync::atomic::AtomicBool::new(false),
            input_starts: std::sync::atomic::AtomicUsize::new(0),
            input_stops: std::sync::atomic::AtomicUsize::new(0),
            usage_starts: std::sync::atomic::AtomicUsize::new(0),
            usage_stops: std::sync::atomic::AtomicUsize::new(0),
            last_keyboard: std::sync::atomic::AtomicBool::new(false),
            last_mouse: std::sync::atomic::AtomicBool::new(false),
            last_screenshot: std::sync::atomic::AtomicBool::new(false),
            last_webcam: std::sync::atomic::AtomicBool::new(false),
            last_perms: Mutex::new(None),
            webcam_captures: std::sync::atomic::AtomicUsize::new(0),
            window_queries: std::sync::atomic::AtomicUsize::new(0),
            process_queries: std::sync::atomic::AtomicUsize::new(0),
            snapshot_calls: std::sync::atomic::AtomicUsize::new(0),
            on_snapshot: Mutex::new(None),
        })
    }

    /// Both clocks move on to `to`.
    fn set(&self, to: DateTime<Utc>) {
        let mut now = self.now.lock().unwrap();
        let elapsed = (to - *now).num_milliseconds().max(0) as u64;
        *self.awake_ms.lock().unwrap() += elapsed;
        *now = to;
    }

    /// The machine sleeps: only the wall clock moves.
    fn sleep_until(&self, to: DateTime<Utc>) {
        *self.now.lock().unwrap() = to;
    }

    fn input_at(&self, when: DateTime<Utc>) {
        self.inputs.lock().unwrap().push(when);
    }

    /// Software-injected input (a mouse jiggler): only Windows' idle clock
    /// sees it.
    fn injected_at(&self, when: DateTime<Utc>) {
        self.injected.lock().unwrap().push(when);
    }

    /// What Windows' idle clock says: the latest input of any kind.
    fn system_last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let real = self.inputs.lock().unwrap().clone();
        let injected = self.injected.lock().unwrap().clone();
        real.into_iter().chain(injected).filter(|t| *t <= now).max()
    }

    /// The hooks see both kinds of input (as the real tracker requires).
    fn hooks_see_all(&self) -> bool {
        self.is_input_running()
            && self.last_keyboard.load(std::sync::atomic::Ordering::SeqCst)
            && self.last_mouse.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn captures(&self) -> u32 {
        *self.captures.lock().unwrap()
    }

    fn is_tracking_active(&self) -> bool {
        self.tracking_active.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_input_running(&self) -> bool {
        self.input_running.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_usage_running(&self) -> bool {
        self.usage_running.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn input_starts(&self) -> usize {
        self.input_starts.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn input_stops(&self) -> usize {
        self.input_stops.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn usage_starts(&self) -> usize {
        self.usage_starts.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn usage_stops(&self) -> usize {
        self.usage_stops.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_screenshot_allowed(&self) -> bool {
        self.last_screenshot.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_webcam_allowed(&self) -> bool {
        self.last_webcam.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn webcam_captures(&self) -> usize {
        self.webcam_captures.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn window_queries(&self) -> usize {
        self.window_queries.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn process_queries(&self) -> usize {
        self.process_queries.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn snapshot_calls(&self) -> usize {
        self.snapshot_calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_on_snapshot<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        *self.on_snapshot.lock().unwrap() = Some(Box::new(f));
    }
}

impl Machine for FakeMachine {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }

    fn awake_millis(&self) -> u64 {
        *self.awake_ms.lock().unwrap()
    }

    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.hooks_see_all() {
            self.inputs.lock().unwrap().iter().copied().filter(|t| *t <= now).max()
        } else {
            self.system_last_input(now)
        }
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.hooks_see_all() {
            // Only what happened while the hooks were attached.
            let attached = self.hooks_since.lock().unwrap().unwrap_or(since);
            self.inputs.lock().unwrap().iter().copied().filter(|t| *t > since && *t >= attached && *t <= now).min()
        } else {
            self.system_last_input(now).filter(|last| *last > since)
        }
    }

    fn input_hooks_running(&self) -> bool {
        self.is_input_running()
    }

    fn capture_screen(&self, blur: bool) -> Option<crate::tracking::ProvenanceScreenCapture> {
        let last_perms = *self.last_perms.lock().unwrap();
        let (authorized, effective_blur) = if self.is_tracking_active() {
            let perms = last_perms.unwrap_or_else(CapturePermissions::all_disabled);
            (perms.screenshot, blur || perms.blur_screenshots)
        } else {
            (false, blur)
        };
        if !authorized {
            return None;
        }
        *self.captures.lock().unwrap() += 1;
        Some(crate::tracking::ProvenanceScreenCapture {
            capture: ScreenCapture {
                jpeg_base64: "/9j/screen".into(),
                thumb_jpg_base64: Some("/9j/dGh1bWI=".into()),
                captured_at: self.now(),
                blurred: effective_blur,
            },
            generation: 1,
        })
    }

    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<crate::tracking::ProvenanceScreenCapture>) -> Activity {
        self.snapshot_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(hook) = self.on_snapshot.lock().unwrap().as_ref() {
            hook();
        }
        let current_perms = if self.is_tracking_active() {
            (*self.last_perms.lock().unwrap()).unwrap_or_else(CapturePermissions::all_disabled)
        } else {
            CapturePermissions::all_disabled()
        };
        let window_now = if current_perms.active_window && block.perms.active_window && self.usage_running.load(std::sync::atomic::Ordering::SeqCst) {
            self.window_queries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(crate::model::WindowInfo {
                app_name: "code.exe".into(),
                window_title: "main.rs".into(),
                seconds: None,
                keyboard_hits: None,
                mouse_clicks: None,
            })
        } else {
            None
        };
        let running_programs = if current_perms.running_programs && block.perms.running_programs {
            self.process_queries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            vec![crate::model::WindowInfo {
                app_name: "code.exe".into(),
                window_title: "code".into(),
                seconds: None,
                keyboard_hits: None,
                mouse_clicks: None,
            }]
        } else {
            Vec::new()
        };
        let screen = if current_perms.screenshot {
            screen.map(|sc| sc.capture).filter(|c| c.blurred || !current_perms.blur_screenshots)
        } else {
            None
        };
        let webcam_jpg_base64 = if current_perms.webcam && block.perms.webcam && block.captures_at_end() {
            self.webcam_captures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some("/9j/synthetic_webcam".into())
        } else {
            None
        };
        let key_presses: Vec<i64> = if block.perms.keyboard {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .copied()
                .filter(|t| *t >= block.started_at && *t <= block.ended_at)
                .map(|t| t.timestamp_millis())
                .collect()
        } else {
            Vec::new()
        };
        let clicks: Vec<i64> = if block.perms.mouse {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .copied()
                .filter(|t| *t >= block.started_at && *t <= block.ended_at)
                .map(|t| t.timestamp_millis())
                .collect()
        } else {
            Vec::new()
        };
        let slices = if block.perms.active_window {
            vec![crate::tracking::usage::UsageSlice {
                app: "code.exe".into(),
                title: "main.rs".into(),
                start_ms: block.started_at.timestamp_millis(),
                end_ms: block.ended_at.timestamp_millis(),
            }]
        } else {
            Vec::new()
        };
        let active_seconds = if !key_presses.is_empty() || !clicks.is_empty() {
            vec![block.started_at.timestamp()]
        } else {
            Vec::new()
        };
        let measured = Measurements {
            key_presses,
            clicks,
            active_seconds,
            injected: 0,
            input_available: true,
            slices,
            window_now,
            running_programs,
            screen,
            webcam_jpg_base64,
        };
        build_activity(block, measured, &self.device, Zone { time_zone: Some("UTC".into()), utc_offset_minutes: 0 })
    }

    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        let interval = TimeDelta::from_std(interval).unwrap();
        crate::tracking::kept_idle_blocks(project_id, from, to, interval, description, &self.device, Some("UTC"))
    }

    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn current_app(&self) -> Option<String> {
        let last_perms = *self.last_perms.lock().unwrap();
        if self.is_tracking_active() && last_perms.is_some_and(|p| p.active_window) {
            Some("code.exe".into())
        } else {
            None
        }
    }

    fn random_fraction(&self) -> f64 {
        *self.fraction.lock().unwrap()
    }

    fn set_tracking_active(&self, active: bool) {
        if active {
            let default_perms = CapturePermissions::default();
            self.apply_sensor_policy(true, Some(&default_perms));
        } else {
            self.apply_sensor_policy(false, None);
        }
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>) {
        self.tracking_active.store(active, std::sync::atomic::Ordering::SeqCst);
        *self.last_perms.lock().unwrap() = perms.cloned();

        let keyboard = active && perms.is_some_and(|p| p.keyboard);
        let mouse = active && perms.is_some_and(|p| p.mouse);
        let active_window = active && perms.is_some_and(|p| p.active_window);
        let screenshot = active && perms.is_some_and(|p| p.screenshot);
        let webcam = active && perms.is_some_and(|p| p.webcam);

        self.last_keyboard.store(keyboard, std::sync::atomic::Ordering::SeqCst);
        self.last_mouse.store(mouse, std::sync::atomic::Ordering::SeqCst);
        self.last_screenshot.store(screenshot, std::sync::atomic::Ordering::SeqCst);
        self.last_webcam.store(webcam, std::sync::atomic::Ordering::SeqCst);

        if keyboard || mouse {
            if !self.input_running.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.input_starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                *self.hooks_since.lock().unwrap() = Some(self.now());
            }
        } else if self.input_running.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.input_stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.hooks_since.lock().unwrap() = None;
        }

        if active_window {
            if !self.usage_running.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.usage_starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        } else if self.usage_running.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.usage_stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// A project as the server lists it: 10-minute interval, idle after
/// `idle_minutes`.
fn project(id: &str, idle_minutes: u32) -> Project {
    serde_json::from_value(serde_json::json!({
        "id": id, "title": format!("Project {id}"), "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true, "allowMouse": true,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": idle_minutes
    }))
    .unwrap()
}

fn project_allowing_all(id: &str, idle_minutes: u32) -> Project {
    serde_json::from_value(serde_json::json!({
        "id": id, "title": format!("Project {id}"), "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": true, "allowKeyboard": true, "allowMouse": true,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": idle_minutes
    }))
    .unwrap()
}

/// A saved sign-in whose project list was resolved an hour ago.
fn saved(projects: Vec<Project>, remembered: Option<&str>) -> Credentials {
    let mut saved = Credentials::new("alice@acme.com".into(), None);
    saved.workspace_id = Some("w1".into());
    saved.workspace_name = "Acme".into();
    saved.project_id = remembered.map(str::to_owned);
    saved.projects = Some(ProjectCache { resolved_at: at("09:00:00"), projects });
    saved
}

struct Harness {
    worker: Worker,
    machine: Arc<FakeMachine>,
    control: UnboundedReceiver<ControlRequest>,
    farewells: UnboundedReceiver<Farewell>,
    uploads: UnboundedReceiver<UploadCommand>,
    state: StateHandle,
    consent_store: Arc<MemoryConsentStore>,
}

fn harness_at(start: DateTime<Utc>, saved: Option<Credentials>, autostart: bool, credentials_file: Option<PathBuf>) -> Harness {
    let machine = FakeMachine::new(start);
    let state = StateHandle::new();
    let (control, control_rx, farewells) = ControlLink::detached();
    let (uploader, uploads) = UploaderHandle::detached();
    let store = Storage::open(":memory:", None).unwrap();
    let consent = crate::config::LocalConsent::Granted(CapturePermissions::default().into());
    let consent_store = Arc::new(MemoryConsentStore::new(consent));
    let cfg = AppConfig { consent, ..AppConfig::default() };
    let mut worker = Worker::new(
        state.clone(),
        cfg,
        machine.clone(),
        Launch { autostart },
        Arc::new(|| {}),
        ServerClock::default(),
        store,
        control,
        uploader,
        credentials_file,
        consent_store.clone(),
    );
    worker.start(saved, None);
    worker.publish();
    Harness { worker, machine, control: control_rx, farewells, uploads, state, consent_store }
}

fn harness_with_consent(start: DateTime<Utc>, saved: Option<Credentials>, consent: LocalConsent) -> Harness {
    let machine = FakeMachine::new(start);
    let state = StateHandle::new();
    let (control, control_rx, farewells) = ControlLink::detached();
    let (uploader, uploads) = UploaderHandle::detached();
    let store = Storage::open(":memory:", None).unwrap();
    let consent_store = Arc::new(MemoryConsentStore::new(consent));
    let cfg = AppConfig {
        consent,
        capture: CapturePermissions { webcam: true, ..CapturePermissions::default() },
        ..AppConfig::default()
    };
    let mut worker = Worker::new(
        state.clone(),
        cfg,
        machine.clone(),
        Launch { autostart: false },
        Arc::new(|| {}),
        ServerClock::default(),
        store,
        control,
        uploader,
        None,
        consent_store.clone(),
    );
    worker.start(saved, None);
    worker.publish();
    Harness { worker, machine, control: control_rx, farewells, uploads, state, consent_store }
}

/// Signed in and tracking "p1" (idle after 5 min) since 10:00.
fn tracking() -> Harness {
    harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), false, None)
}

impl Harness {
    fn tick(&mut self, now: DateTime<Utc>) {
        self.machine.set(now);
        self.worker.on_tick();
        self.worker.publish();
    }

    fn command(&mut self, now: DateTime<Utc>, command: Command) {
        self.machine.set(now);
        self.worker.on_command(command);
        self.worker.publish();
    }

    fn system(&mut self, event: SystemEvent, when: DateTime<Utc>) {
        self.machine.set(when.max(self.machine.now()));
        self.worker.on_system(SystemNotice { event, at: when, done: None });
        self.worker.publish();
    }

    fn status(&self) -> TrackerStatus {
        self.state.snapshot().status
    }

    fn owner(&self) -> String {
        storage::owner_key("https://localhost:6006", "alice@acme.com")
    }

    /// The queued blocks, oldest first.
    fn queued(&self) -> Vec<Activity> {
        self.worker.store.pending_batch(&self.owner(), 0, 1_000).unwrap().into_iter().map(|(_, a)| a).collect()
    }

    fn spans(&self) -> Vec<(DateTime<Utc>, DateTime<Utc>, Option<EndReason>)> {
        self.queued().into_iter().map(|a| (a.started_at, a.ended_at, a.end_reason)).collect()
    }

    fn control_requests(&mut self) -> Vec<ControlRequest> {
        std::iter::from_fn(|| self.control.try_recv().ok()).collect()
    }

    fn upload_commands(&mut self) -> Vec<UploadCommand> {
        std::iter::from_fn(|| self.uploads.try_recv().ok()).collect()
    }

    /// The statuses of the heartbeats sent since the last look.
    fn heartbeats(&mut self) -> Vec<PresenceStatus> {
        self.control_requests()
            .into_iter()
            .filter_map(|request| match request {
                ControlRequest::Heartbeat(heartbeat) => Some(heartbeat.status),
                _ => None,
            })
            .collect()
    }

    fn tracking_since(&self) -> Option<DateTime<Utc>> {
        self.state.snapshot().tracking_since
    }

    /// The user comes back at `when` after being away: Windows' idle clock
    /// sees it on the next tick, the input hooks are attached to confirm it,
    /// and they see the user still at it a moment later.
    fn come_back(&mut self, when: DateTime<Utc>) {
        self.machine.input_at(when);
        self.tick(when + TimeDelta::seconds(1));
        self.machine.input_at(when + TimeDelta::seconds(2));
        self.tick(when + TimeDelta::seconds(3));
    }
}

// ---- Signing in and choosing the workspace -----------------------------------------

fn workspace(id: &str, name: &str) -> Workspace {
    Workspace { id: id.into(), name: name.into(), role: Some("Member".into()), plan_name: None, is_owner: false, is_archived: false }
}

/// Type the email and password; the server accepts them.
fn typed_sign_in(h: &mut Harness, workspace: Option<&str>) -> Account {
    h.command(at("10:00:00"), Command::SignIn {
        workspace: workspace.map(Into::into),
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: None,
    });
    assert!(h.state.snapshot().signing_in);
    match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => {
            assert_eq!((account.username.as_str(), account.password.as_str(), account.workspace_id.as_deref()), ("alice@acme.com", "pw", None));
            account
        }
        other => panic!("expected a sign-in check, got {other:?}"),
    }
}

/// The account the worker now uses, as handed to the control task.
fn used_account(h: &mut Harness) -> Account {
    h.control_requests()
        .into_iter()
        .rev()
        .find_map(|r| match r {
            ControlRequest::UseAccount(Some(account)) => Some(account),
            _ => None,
        })
        .expect("an account in use")
}
