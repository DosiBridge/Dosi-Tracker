//! The worker, driven through a fake machine (ported from the Windows
//! `worker/tests.rs`), plus the Linux cases of design 5.2 and 6.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use chrono::FixedOffset;

use super::control::Farewell;
use super::*;
use crate::agent::state::SessionKind;
use crate::agent::state::SharedState;
use crate::consent::{
    ConsentError, ConsentManager, ConsentStorage, ConsentStore, MemoryConsentStore,
};
use crate::credentials::InMemoryCredentialStore;
use crate::lifecycle::{LifecycleGeneration, TrackerState};
use crate::tracking::build::{build_activity, Measurements, Zone};
use crate::tracking::model::WindowInfo;
use crate::tracking::tracker::SensorDriver;
use crate::tracking::usage::UsageSlice;

const SERVER: &str = "https://tracker.acme.test";
const DEVICE_ID: &str = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11";
const KEY: [u8; 32] = [7u8; 32];

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// 5 October 2026 at `hms`, UTC.
fn at(hms: &str) -> DateTime<Utc> {
    utc(&format!("2026-10-05T{hms}Z"))
}

/// A stable GUID for a short test name: the server and the outbox take
/// GUIDs only.
fn guid(name: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let low = hash.rotate_left(17);
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        (hash >> 32) as u32,
        (hash >> 16) as u16,
        hash & 0xfff,
        (low >> 20) & 0xfff,
        low & 0xffff_ffff_ffff
    )
}

fn pid(name: &str) -> String {
    guid(&format!("project-{name}"))
}

fn wid(name: &str) -> String {
    guid(&format!("workspace-{name}"))
}

/// What the Windows tests' `CapturePermissions::default()` grants:
/// everything but the webcam, no blur.
fn defaults() -> ConsentedPermissions {
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

// ---- Local days and totals ------------------------------------------------------------

#[test]
fn today_starts_at_local_midnight_ahead_of_utc() {
    // 02:00 on Thursday 24 Sep in UTC+6 is still Wednesday in UTC.
    let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
    let (today, week) = local_day_and_week_start(utc("2026-09-23T20:00:00Z"), &dhaka);
    assert_eq!(today, utc("2026-09-23T18:00:00Z")); // Thu 24 Sep 00:00 +06
    assert_eq!(week, utc("2026-09-20T18:00:00Z")); // Mon 21 Sep 00:00 +06
}

#[test]
fn today_starts_at_local_midnight_behind_utc() {
    // 22:00 on Sunday 27 Sep in UTC-5 is already Monday in UTC.
    let bogota = FixedOffset::west_opt(5 * 3600).unwrap();
    let (today, week) = local_day_and_week_start(utc("2026-09-28T03:00:00Z"), &bogota);
    assert_eq!(today, utc("2026-09-27T05:00:00Z")); // Sun 27 Sep 00:00 -05
    assert_eq!(week, utc("2026-09-21T05:00:00Z")); // Mon 21 Sep 00:00 -05
}

#[test]
fn utc_zone_keeps_utc_midnights() {
    let (today, week) = local_day_and_week_start(utc("2026-09-23T12:34:56Z"), &Utc);
    assert_eq!(today, utc("2026-09-23T00:00:00Z"));
    assert_eq!(week, utc("2026-09-21T00:00:00Z"));
}

#[test]
fn past_days_of_the_week_are_local_days_before_today() {
    let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
    // Thursday 24 Sep, 02:00 in UTC+6: Monday to Wednesday are past.
    let days = past_days_of_week(utc("2026-09-23T20:00:00Z"), &dhaka);
    assert_eq!(days.len(), 3);
    assert_eq!(
        days[0],
        (0, utc("2026-09-20T18:00:00Z"), utc("2026-09-21T18:00:00Z"))
    );
    assert_eq!(
        days[2],
        (2, utc("2026-09-22T18:00:00Z"), utc("2026-09-23T18:00:00Z"))
    );

    // On a Monday nothing is past yet.
    assert!(past_days_of_week(utc("2026-09-21T09:00:00Z"), &Utc).is_empty());
    // On a Sunday, six days are.
    assert_eq!(
        past_days_of_week(utc("2026-09-27T09:00:00Z"), &Utc).len(),
        6
    );
}

#[test]
fn only_the_part_of_a_block_after_the_cutoff_is_counted() {
    let midnight = utc("2026-09-30T18:00:00Z");
    // Entirely after: all of it.
    assert_eq!(
        tracked_secs_since(
            utc("2026-09-30T18:10:00Z"),
            utc("2026-09-30T18:15:00Z"),
            midnight
        ),
        300.0
    );
    // Spanning midnight: only the part after it.
    assert_eq!(
        tracked_secs_since(
            utc("2026-09-30T17:58:00Z"),
            utc("2026-09-30T18:03:30Z"),
            midnight
        ),
        210.0
    );
    // Entirely before: nothing.
    assert_eq!(
        tracked_secs_since(
            utc("2026-09-30T17:00:00Z"),
            utc("2026-09-30T17:05:00Z"),
            midnight
        ),
        0.0
    );
}

// ---- Signing in ------------------------------------------------------------------------

#[test]
fn only_a_refusal_counts_as_a_rejected_sign_in() {
    // Anything but the server refusing the credentials — here a plain
    // failure, as when the server cannot be reached — keeps the sign-in.
    assert!(!AuthError::Offline("tcp connect error".into()).is_rejected());
}

/// Serve one HTTP response with `status` and `body` on a local port,
/// returning its URL.
fn one_shot_server(status: &'static str, body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

#[tokio::test]
async fn refused_credentials_are_a_rejected_sign_in() {
    let refused =
        r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
    let mut client = ApiClient::new(
        one_shot_server("400 Bad Request", refused),
        "a@b.c".into(),
        "x".into(),
    );
    let error = client.login().await.expect_err("the server refuses");
    assert!(error.is_rejected());
    assert_eq!(auth_message(&error), "Invalid username or password!");
}

#[tokio::test]
async fn a_server_error_is_not_a_rejected_sign_in() {
    let mut client = ApiClient::new(
        one_shot_server("503 Service Unavailable", ""),
        "a@b.c".into(),
        "x".into(),
    );
    let error = client.login().await.expect_err("the server fails");
    assert!(!error.is_rejected());
}

#[tokio::test]
async fn an_unreachable_server_is_not_a_rejected_sign_in() {
    // Nothing listens on port 9 (discard) locally: a real connect error.
    let mut client = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("nothing is listening");
    assert!(!error.is_rejected());
}

#[test]
fn accounts_never_show_their_password() {
    let mut account = Account::new("https://t.example".into(), "a@acme.com".into(), "s3cr3t")
        .in_workspace("w1", "Acme");
    account.signing_key = Some(KEY);
    let shown = format!("{account:?}");
    assert!(
        !shown.contains("s3cr3t") && shown.contains("a@acme.com"),
        "{shown}"
    );
    assert!(!shown.contains("7, 7"), "nor the signing key: {shown}");
}

// ---- The clients' lists ------------------------------------------------------------------

fn block(row_id: i64) -> RecentBlock {
    let at = utc("2026-09-30T10:00:00Z");
    RecentBlock {
        row_id,
        project: "Website".into(),
        started_at: at,
        ended_at: at,
        productivity: Some(50),
        keyboard_hits: Some(0),
        mouse_clicks: None,
        screenshot: false,
        upload: UploadState::Queued,
    }
}

#[test]
fn recent_blocks_are_newest_first_and_bounded() {
    let mut blocks = Vec::new();
    for id in 1..=(RECENT_BLOCKS as i64 + 3) {
        push_recent_block(&mut blocks, block(id));
    }
    assert_eq!(blocks.len(), RECENT_BLOCKS);
    assert_eq!(blocks[0].row_id, RECENT_BLOCKS as i64 + 3);
    assert_eq!(blocks.last().unwrap().row_id, 4);
}

#[test]
fn upload_results_are_matched_to_their_block() {
    let state = StateHandle::new();
    state.update(|s| {
        push_recent_block(&mut s.recent_blocks, block(1));
        push_recent_block(&mut s.recent_blocks, block(2));
    });
    set_upload_state(&state, 1, UploadState::Uploaded);
    set_upload_state(&state, 2, UploadState::Rejected);
    set_upload_state(&state, 99, UploadState::Uploaded); // not listed: ignored
    let blocks = state.snapshot().recent_blocks;
    assert_eq!(blocks[0].upload, UploadState::Rejected);
    assert_eq!(blocks[1].upload, UploadState::Uploaded);
}

#[test]
fn task_notes_are_one_tidy_line_of_at_most_two_hundred_characters() {
    assert_eq!(
        clean_note("  Invoice   export\n\tfor ACME \u{7} "),
        Some("Invoice export for ACME".into())
    );
    assert_eq!(clean_note(" \n "), None);
    let long = "word ".repeat(100);
    let kept = clean_note(&long).unwrap();
    assert!(kept.chars().count() <= TASK_NOTE_MAX_CHARS);
    assert!(!kept.ends_with(' '));
}

// ---- The worker, driven through a fake machine -----------------------------------------

/// A machine whose clocks, input, screen and session the test controls.
struct FakeMachine {
    now: Mutex<DateTime<Utc>>,
    awake_ms: Mutex<u64>,
    inputs: Mutex<Vec<DateTime<Utc>>>,
    captures: Mutex<u32>,
    fraction: Mutex<f64>,
    device: DeviceInfo,
    tracking_active: AtomicBool,
    input_running: AtomicBool,
    usage_running: AtomicBool,
    input_starts: AtomicUsize,
    input_stops: AtomicUsize,
    usage_starts: AtomicUsize,
    usage_stops: AtomicUsize,
    last_keyboard: AtomicBool,
    last_mouse: AtomicBool,
    last_screenshot: AtomicBool,
    last_perms: Mutex<Option<ConsentedPermissions>>,
    window_queries: AtomicUsize,
    process_queries: AtomicUsize,
    snapshot_calls: AtomicUsize,
    on_snapshot: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    sink: Mutex<Option<UnboundedSender<CaptureEvent>>>,
    /// Hold the asynchronous captures instead of delivering them.
    hold_captures: AtomicBool,
    held: Mutex<Vec<CaptureEvent>>,
    summary: Mutex<SessionSummary>,
    platform: Mutex<PlatformCapabilities>,
    capture_retries: AtomicUsize,
}

impl FakeMachine {
    fn new(start: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(start),
            awake_ms: Mutex::new(1_000_000),
            inputs: Mutex::new(Vec::new()),
            captures: Mutex::new(0),
            fraction: Mutex::new(0.5),
            device: DeviceInfo {
                device_id: DEVICE_ID.into(),
                device_name: Some("work-laptop".into()),
                os_version: "Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)".into(),
                platform: "linux".into(),
                agent_version: "9.9.9".into(),
                time_zone: Some("UTC".into()),
            },
            tracking_active: AtomicBool::new(false),
            input_running: AtomicBool::new(false),
            usage_running: AtomicBool::new(false),
            input_starts: AtomicUsize::new(0),
            input_stops: AtomicUsize::new(0),
            usage_starts: AtomicUsize::new(0),
            usage_stops: AtomicUsize::new(0),
            last_keyboard: AtomicBool::new(false),
            last_mouse: AtomicBool::new(false),
            last_screenshot: AtomicBool::new(false),
            last_perms: Mutex::new(None),
            window_queries: AtomicUsize::new(0),
            process_queries: AtomicUsize::new(0),
            snapshot_calls: AtomicUsize::new(0),
            on_snapshot: Mutex::new(None),
            sink: Mutex::new(None),
            hold_captures: AtomicBool::new(false),
            held: Mutex::new(Vec::new()),
            summary: Mutex::new(SessionSummary {
                kind: SessionKind::X11,
                desktop: Some("GNOME".into()),
                idle_source: Some("xi2".into()),
            }),
            platform: Mutex::new(PlatformCapabilities::everything()),
            capture_retries: AtomicUsize::new(0),
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

    fn captures(&self) -> u32 {
        *self.captures.lock().unwrap()
    }

    fn is_tracking_active(&self) -> bool {
        self.tracking_active.load(Ordering::SeqCst)
    }

    fn is_input_running(&self) -> bool {
        self.input_running.load(Ordering::SeqCst)
    }

    fn is_usage_running(&self) -> bool {
        self.usage_running.load(Ordering::SeqCst)
    }

    fn input_starts(&self) -> usize {
        self.input_starts.load(Ordering::SeqCst)
    }

    fn input_stops(&self) -> usize {
        self.input_stops.load(Ordering::SeqCst)
    }

    fn usage_starts(&self) -> usize {
        self.usage_starts.load(Ordering::SeqCst)
    }

    fn usage_stops(&self) -> usize {
        self.usage_stops.load(Ordering::SeqCst)
    }

    fn keyboard(&self) -> bool {
        self.last_keyboard.load(Ordering::SeqCst)
    }

    fn mouse(&self) -> bool {
        self.last_mouse.load(Ordering::SeqCst)
    }

    fn is_screenshot_allowed(&self) -> bool {
        self.last_screenshot.load(Ordering::SeqCst)
    }

    fn window_queries(&self) -> usize {
        self.window_queries.load(Ordering::SeqCst)
    }

    fn process_queries(&self) -> usize {
        self.process_queries.load(Ordering::SeqCst)
    }

    fn snapshot_calls(&self) -> usize {
        self.snapshot_calls.load(Ordering::SeqCst)
    }

    fn set_on_snapshot<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        *self.on_snapshot.lock().unwrap() = Some(Box::new(f));
    }

    /// No idle source: manual mode (design 5.2).
    fn set_manual(&self) {
        self.summary.lock().unwrap().idle_source = None;
    }

    fn set_session(&self, kind: SessionKind) {
        self.summary.lock().unwrap().kind = kind;
        *self.platform.lock().unwrap() = match kind {
            SessionKind::X11 => PlatformCapabilities::everything(),
            _ => PlatformCapabilities::nothing("No graphical session is attached."),
        };
    }

    fn manual(&self) -> bool {
        self.summary.lock().unwrap().idle_source.is_none()
    }

    fn current_perms(&self) -> ConsentedPermissions {
        if self.is_tracking_active() {
            (*self.last_perms.lock().unwrap()).unwrap_or_else(ConsentedPermissions::all_disabled)
        } else {
            ConsentedPermissions::all_disabled()
        }
    }

    fn shot(&self, blurred: bool) -> ProvenanceScreenCapture {
        ProvenanceScreenCapture {
            capture: ScreenCapture {
                jpeg: vec![0xff, 0xd8, 0xff, 0x01],
                thumb_jpeg: Some(vec![0xff, 0xd8, 0xff, 0x02]),
                captured_at: self.now(),
                blurred,
            },
            generation: LifecycleGeneration(1),
        }
    }

    /// Deliver the held asynchronous captures.
    fn release_captures(&self) {
        let held: Vec<CaptureEvent> = std::mem::take(&mut *self.held.lock().unwrap());
        let sink = self.sink.lock().unwrap().clone().unwrap();
        for event in held {
            sink.send(event).unwrap();
        }
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
        if self.manual() {
            return Some(now);
        }
        self.inputs
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|t| *t <= now)
            .max()
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.manual() {
            return None;
        }
        self.inputs
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|t| *t > since && *t <= now)
            .min()
    }

    fn set_capture_sink(&self, sink: UnboundedSender<CaptureEvent>) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    fn request_capture(&self, block: BlockId, blur: bool) -> bool {
        let perms = self.current_perms();
        if !perms.screenshot {
            return false;
        }
        *self.captures.lock().unwrap() += 1;
        let event = CaptureEvent {
            block_id: block,
            capture: Some(self.shot(blur || perms.blur_screenshots)),
        };
        if self.hold_captures.load(Ordering::SeqCst) {
            self.held.lock().unwrap().push(event);
        } else if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            let _ = sink.send(event);
        }
        true
    }

    fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        let perms = self.current_perms();
        if !perms.screenshot {
            return None;
        }
        *self.captures.lock().unwrap() += 1;
        Some(self.shot(blur || perms.blur_screenshots))
    }

    fn snapshot(
        &self,
        block: &BlockSpec<'_>,
        screen: Option<ProvenanceScreenCapture>,
    ) -> BuiltBlock {
        self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_snapshot.lock().unwrap().as_ref() {
            hook();
        }
        let current_perms = self.current_perms();
        let window_now = if current_perms.active_window
            && block.perms.active_window
            && self.usage_running.load(Ordering::SeqCst)
        {
            self.window_queries.fetch_add(1, Ordering::SeqCst);
            Some(WindowInfo {
                app_name: "code".into(),
                window_title: "main.rs".into(),
                ..Default::default()
            })
        } else {
            None
        };
        let running_programs = if current_perms.running_programs && block.perms.running_programs {
            self.process_queries.fetch_add(1, Ordering::SeqCst);
            vec![WindowInfo {
                app_name: "code".into(),
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        let screen = if current_perms.screenshot {
            screen
                .map(|sc| sc.capture)
                .filter(|c| c.blurred || !current_perms.blur_screenshots)
        } else {
            None
        };
        let inputs: Vec<i64> = self
            .inputs
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|t| *t >= block.started_at && *t <= block.ended_at)
            .map(|t| t.timestamp_millis())
            .collect();
        let active_seconds = if !inputs.is_empty() && (block.perms.keyboard || block.perms.mouse) {
            vec![block.started_at.timestamp()]
        } else {
            Vec::new()
        };
        let slices = if block.perms.active_window {
            vec![UsageSlice {
                app: "code".into(),
                title: "main.rs".into(),
                start_ms: block.started_at.timestamp_millis(),
                end_ms: block.ended_at.timestamp_millis(),
            }]
        } else {
            Vec::new()
        };
        let measured = Measurements {
            key_presses: block.perms.keyboard.then(|| inputs.clone()),
            clicks: block.perms.mouse.then(|| inputs.clone()),
            active_seconds,
            injected: 0,
            input_available: true,
            slices,
            window_now,
            running_programs,
            screen,
        };
        build_activity(
            block,
            measured,
            &self.device,
            Zone {
                time_zone: Some("UTC".into()),
                utc_offset_minutes: 0,
            },
        )
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
        crate::tracking::build::kept_idle_blocks(
            project_id,
            from,
            to,
            interval,
            description,
            &self.device,
            |_| Zone {
                time_zone: Some("UTC".into()),
                utc_offset_minutes: 0,
            },
        )
    }

    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn current_app(&self) -> Option<String> {
        self.current_perms().active_window.then(|| "code".into())
    }

    fn random_fraction(&self) -> f64 {
        *self.fraction.lock().unwrap()
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&ConsentedPermissions>) {
        self.tracking_active.store(active, Ordering::SeqCst);
        *self.last_perms.lock().unwrap() = perms.cloned();

        let keyboard = active && perms.is_some_and(|p| p.keyboard);
        let mouse = active && perms.is_some_and(|p| p.mouse);
        let active_window = active && perms.is_some_and(|p| p.active_window);
        let screenshot = active && perms.is_some_and(|p| p.screenshot);

        self.last_keyboard.store(keyboard, Ordering::SeqCst);
        self.last_mouse.store(mouse, Ordering::SeqCst);
        self.last_screenshot.store(screenshot, Ordering::SeqCst);

        if keyboard || mouse {
            if !self.input_running.swap(true, Ordering::SeqCst) {
                self.input_starts.fetch_add(1, Ordering::SeqCst);
            }
        } else if self.input_running.swap(false, Ordering::SeqCst) {
            self.input_stops.fetch_add(1, Ordering::SeqCst);
        }

        if active_window {
            if !self.usage_running.swap(true, Ordering::SeqCst) {
                self.usage_starts.fetch_add(1, Ordering::SeqCst);
            }
        } else if self.usage_running.swap(false, Ordering::SeqCst) {
            self.usage_stops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn session_summary(&self) -> SessionSummary {
        self.summary.lock().unwrap().clone()
    }

    fn platform(&self) -> PlatformCapabilities {
        self.platform.lock().unwrap().clone()
    }

    fn retry_capture_permission(&self) {
        self.capture_retries.fetch_add(1, Ordering::SeqCst);
    }
}

/// A project as the server lists it: 10-minute interval, idle after
/// `idle_minutes`.
fn project(name: &str, idle_minutes: u32) -> Project {
    serde_json::from_value(serde_json::json!({
        "id": pid(name), "title": format!("Project {name}"), "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true, "allowMouse": true,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": idle_minutes
    }))
    .unwrap()
}

fn project_allowing_all(name: &str, idle_minutes: u32) -> Project {
    serde_json::from_value(serde_json::json!({
        "id": pid(name), "title": format!("Project {name}"), "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": true, "allowKeyboard": true, "allowMouse": true,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": idle_minutes
    }))
    .unwrap()
}

/// A saved sign-in: `session.json` and the keyring item.
#[derive(Clone)]
struct Saved {
    session: SessionData,
    secrets: Credentials,
}

/// A saved sign-in whose project list was resolved an hour ago.
fn saved(projects: Vec<Project>, remembered: Option<&str>) -> Saved {
    let session = SessionData {
        server: Some(SERVER.into()),
        account: Some("alice@acme.com".into()),
        workspace: Some(SavedWorkspace {
            id: wid("w1"),
            name: "Acme".into(),
        }),
        project_id: remembered.map(pid),
        projects: Some(ProjectCache {
            resolved_at: at("09:00:00"),
            projects,
        }),
        ..SessionData::default()
    };
    let mut secrets = Credentials::new("alice@acme.com", None);
    secrets.server_url = Some(SERVER.into());
    secrets.refresh_token = Some("refresh-1".into());
    secrets.device_signing_key = Some(KEY.to_vec());
    secrets.device_id = Some(DEVICE_ID.into());
    Saved { session, secrets }
}

struct Setup {
    start: DateTime<Utc>,
    saved: Option<Saved>,
    launch: Launch,
    session_file: Option<PathBuf>,
    consent: LocalConsent,
    cfg: AppConfig,
    credentials: Arc<InMemoryCredentialStore>,
    machine: Option<Arc<dyn Machine>>,
    lifecycle: Option<Arc<Mutex<TrackerLifecycle>>>,
    fake: Option<Arc<FakeMachine>>,
}

fn setup(start: DateTime<Utc>, saved: Option<Saved>) -> Setup {
    Setup {
        start,
        saved,
        launch: Launch::default(),
        session_file: None,
        consent: LocalConsent::Granted(defaults()),
        cfg: AppConfig {
            api_base_url: Some(SERVER.into()),
            ..AppConfig::default()
        },
        credentials: Arc::new(InMemoryCredentialStore::new()),
        machine: None,
        lifecycle: None,
        fake: None,
    }
}

struct Harness {
    worker: Worker,
    machine: Arc<FakeMachine>,
    control: UnboundedReceiver<ControlRequest>,
    farewells: UnboundedReceiver<Farewell>,
    uploads: std::sync::mpsc::Receiver<UploadCommand>,
    captures: UnboundedReceiver<CaptureEvent>,
    state: StateHandle,
    consent_store: Arc<MemoryConsentStore>,
    credentials: Arc<InMemoryCredentialStore>,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
    _dir: tempfile::TempDir,
}

fn build(setup: Setup) -> Harness {
    let fake = setup.fake.unwrap_or_else(|| FakeMachine::new(setup.start));
    let machine: Arc<dyn Machine> = setup.machine.unwrap_or_else(|| fake.clone());
    let state = StateHandle::new();
    let (control, control_rx, farewells) = ControlLink::detached();
    let (uploader, uploads) = UploaderHandle::detached();
    let dir = tempfile::tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let consent_store = Arc::new(MemoryConsentStore::new(setup.consent));
    let lifecycle = setup.lifecycle.unwrap_or_else(|| {
        // The daemon loads the consent once; the worker never touches disk.
        let initial = consent_store.load().unwrap_or_default();
        Arc::new(Mutex::new(TrackerLifecycle::new(
            ConsentManager::with_storage(initial, Some(consent_store.clone())),
        )))
    });
    if let Some(saved) = setup.saved.as_ref() {
        if setup
            .credentials
            .load_account(&saved.secrets.account_id)
            .ok()
            .flatten()
            .is_none()
        {
            setup.credentials.save_account(&saved.secrets).unwrap();
        }
    }
    let (capture_tx, captures) = mpsc::unbounded_channel();
    machine.set_capture_sink(capture_tx);
    let mut worker = Worker::new(WorkerParts {
        state: state.clone(),
        cfg: setup.cfg,
        machine,
        lifecycle: lifecycle.clone(),
        credentials: setup.credentials.clone(),
        session_file: setup.session_file,
        launch: setup.launch,
        clock: ServerClock::default(),
        outbox,
        control,
        uploader,
    });
    worker.start(setup.saved.map(|s| s.session), None);
    worker.publish();
    Harness {
        worker,
        machine: fake,
        control: control_rx,
        farewells,
        uploads,
        captures,
        state,
        consent_store,
        credentials: setup.credentials,
        lifecycle,
        _dir: dir,
    }
}

fn harness_at(
    start: DateTime<Utc>,
    saved: Option<Saved>,
    autostart: bool,
    session_file: Option<PathBuf>,
) -> Harness {
    let mut s = setup(start, saved);
    s.launch.autostart = autostart;
    s.session_file = session_file;
    build(s)
}

fn harness_with_consent(
    start: DateTime<Utc>,
    saved: Option<Saved>,
    consent: LocalConsent,
) -> Harness {
    let mut s = setup(start, saved);
    s.consent = consent;
    build(s)
}

/// Signed in and tracking "p1" (idle after 5 min) since 10:00.
fn tracking() -> Harness {
    harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], None)),
        false,
        None,
    )
}

impl Harness {
    /// Hand the worker the screenshots `dosi-capture` returned.
    fn deliver_captures(&mut self) {
        while let Ok(event) = self.captures.try_recv() {
            self.worker.on_capture(event);
        }
    }

    fn tick(&mut self, now: DateTime<Utc>) {
        self.machine.set(now);
        self.worker.on_tick();
        self.deliver_captures();
        self.worker.publish();
    }

    fn command(&mut self, now: DateTime<Utc>, command: Command) {
        self.machine.set(now);
        self.worker.on_command(command);
        self.deliver_captures();
        self.worker.publish();
    }

    fn system(&mut self, event: SystemEvent, when: DateTime<Utc>) {
        self.machine.set(when.max(self.machine.now()));
        self.worker.on_system(SystemNotice {
            event,
            at: when,
            done: None,
        });
        self.worker.publish();
    }

    fn status(&self) -> TrackerStatus {
        self.state.snapshot().status
    }

    fn owner(&self) -> String {
        owner_key("alice@acme.com")
    }

    /// The queued rows with their workspace, oldest first, as uploaded
    /// (captures attached).
    fn rows(&self) -> Vec<(String, Activity)> {
        let store = &self.worker.store;
        let mut entries = Vec::new();
        for scope in store.pending_scopes(&self.owner()).unwrap() {
            for entry in store.peek_ready(&scope, 200).unwrap() {
                let captures = store.load_captures(&scope, entry.id).unwrap();
                let activity = crate::api::hydrate_v2_activity(
                    entry.payload_json.as_deref().unwrap(),
                    &captures,
                    entry.payload_signature.as_deref(),
                )
                .unwrap();
                entries.push((entry.id, scope.workspace_id.clone(), activity));
            }
        }
        entries.sort_by_key(|(id, _, _)| *id);
        entries.into_iter().map(|(_, ws, a)| (ws, a)).collect()
    }

    /// The queued blocks, oldest first.
    fn queued(&self) -> Vec<Activity> {
        self.rows().into_iter().map(|(_, a)| a).collect()
    }

    fn spans(&self) -> Vec<(DateTime<Utc>, DateTime<Utc>, Option<EndReason>)> {
        self.queued()
            .into_iter()
            .map(|a| (a.started_at, a.ended_at, a.end_reason))
            .collect()
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

    fn notices(&self) -> Vec<NoticeKind> {
        self.state
            .snapshot()
            .notices
            .into_iter()
            .map(|n| n.kind)
            .collect()
    }
}

#[test]
fn a_saved_sign_in_starts_tracking_at_once_on_the_saved_projects() {
    let mut h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p2"))),
        false,
        None,
    );
    assert_eq!(
        h.status(),
        TrackerStatus::Tracking,
        "offline first: no waiting for the server"
    );
    let snapshot = h.state.snapshot();
    assert_eq!(
        snapshot.selected_project,
        Some(pid("p2")),
        "the remembered project"
    );
    assert_eq!(snapshot.tracking_since, Some(at("10:00:00")));
    assert_eq!(snapshot.projects.len(), 2);
    assert_eq!(snapshot.display_name, "alice@acme.com");

    let requests = h.control_requests();
    assert!(
        matches!(&requests[0], ControlRequest::UseAccount(Some(account)) if account.username == "alice@acme.com")
    );
    assert!(
        matches!(requests[1], ControlRequest::RefreshProjects),
        "re-validated with the server"
    );
    assert!(requests.iter().any(
        |r| matches!(r, ControlRequest::Heartbeat(h) if h.status == PresenceStatus::Tracking)
    ));
    assert!(matches!(
        &h.upload_commands()[0],
        UploadCommand::Account(Some(_))
    ));
}

#[test]
fn several_projects_and_none_remembered_ask_the_user_to_choose() {
    let mut h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("acme", 5), project("zeta", 5)], None)),
        false,
        None,
    );
    assert_eq!(
        h.status(),
        TrackerStatus::ChooseProject,
        "never silently the first one"
    );
    assert!(h.tracking_since().is_none());
    assert!(h.notices().contains(&NoticeKind::ChooseProject));
    assert!(h.heartbeats().contains(&PresenceStatus::Choosing));

    h.command(
        at("10:01:00"),
        Command::SelectProject {
            project_id: pid("zeta"),
            at: at("10:01:00"),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:01:00")));
    assert_eq!(
        h.worker.saved.as_ref().unwrap().project_id,
        Some(pid("zeta")),
        "remembered"
    );
}

#[test]
fn without_a_recent_project_list_it_waits_for_the_server() {
    let mut old = saved(vec![project("p1", 5)], Some("p1"));
    old.session.projects.as_mut().unwrap().resolved_at = at("10:00:00") - TimeDelta::days(8);
    let mut h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(
        h.status(),
        TrackerStatus::Connecting,
        "a list older than a week is not used"
    );

    h.worker.on_control(ControlEvent::Reachable(false));
    h.worker
        .on_control(ControlEvent::ProjectsFailed(ApiError::Offline(
            "tcp connect error".into(),
        )));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Offline);

    h.machine.set(at("10:03:00"));
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:03:00")));
    assert!(h.state.snapshot().server_reachable);
    // The fresh list is saved for the next offline start.
    assert_eq!(
        h.worker
            .saved
            .as_ref()
            .unwrap()
            .projects
            .as_ref()
            .unwrap()
            .resolved_at,
        at("10:03:00")
    );
}

fn session_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state").join("session.json");
    (dir, file)
}

fn load_session(file: &std::path::Path) -> SessionData {
    session_store::load(file, Utc::now()).unwrap().data
}

#[test]
fn the_block_ends_where_the_user_paused_and_the_pause_survives_a_restart() {
    let (_dir, file) = session_file();
    let first = saved(vec![project("p1", 5)], None);
    session_store::save(&file, &first.session).unwrap();
    let credentials = Arc::new(InMemoryCredentialStore::new());
    let mut s = setup(
        at("10:00:00"),
        Some(Saved {
            session: load_session(&file),
            ..first.clone()
        }),
    );
    s.session_file = Some(file.clone());
    s.credentials = credentials.clone();
    let mut h = build(s);
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));

    // Clicked at 10:05; the worker got to it two seconds later.
    h.command(
        at("10:05:02"),
        Command::Pause {
            resume_after: None,
            at: at("10:05:00"),
        },
    );
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:05:00"), Some(EndReason::Pause))]
    );
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(h.state.snapshot().paused_since, Some(at("10:05:00")));
    assert!(matches!(
        h.upload_commands().last(),
        Some(UploadCommand::RowsQueued)
    ));

    // Restarted by the login autostart: still paused.
    let mut s = setup(
        at("18:00:00"),
        Some(Saved {
            session: load_session(&file),
            ..first
        }),
    );
    s.session_file = Some(file.clone());
    s.credentials = credentials;
    s.launch.autostart = true;
    let mut restarted = build(s);
    assert_eq!(restarted.status(), TrackerStatus::Paused);
    assert!(restarted.tracking_since().is_none());
    assert!(restarted.heartbeats().contains(&PresenceStatus::Paused));
    restarted.command(at("18:01:00"), Command::Resume { at: at("18:01:00") });
    assert_eq!(
        restarted.tracking_since(),
        Some(at("18:01:00")),
        "a fresh block from the click"
    );
    assert!(load_session(&file).pause.is_none(), "resuming is saved too");
}

#[test]
fn a_timed_break_never_resumes_silently_after_an_autostart() {
    let mut on_break = saved(vec![project("p1", 5)], None);
    on_break.session.pause = Some(SavedPause {
        since: at("09:50:00"),
        until: Some(at("10:05:00")),
    });

    let mut h = harness_at(at("10:00:00"), Some(on_break.clone()), true, None);
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(
        h.state.snapshot().resume_at,
        None,
        "paused until the user resumes"
    );
    h.machine.input_at(at("10:06:00"));
    h.tick(at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Paused);

    // Opened by the user instead: the break goes on and ends by itself.
    let mut h = harness_at(at("10:00:00"), Some(on_break), false, None);
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:05:00")));
    assert!(h.heartbeats().contains(&PresenceStatus::Break));
    h.machine.input_at(at("10:04:00"));
    h.tick(at("10:05:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(
        h.tracking_since(),
        Some(at("10:05:00")),
        "from the end of the break"
    );
}

#[test]
fn a_break_that_ends_while_the_user_is_away_resumes_with_their_return() {
    let mut h = tracking();
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: Some(Duration::from_secs(15 * 60)),
            at: at("10:05:00"),
        },
    );
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:05:00"), Some(EndReason::Break))]
    );
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:20:00")));

    // Nobody touched the computer: the break ends, tracking waits.
    h.tick(at("10:20:01"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(h.tracking_since().is_none());

    h.machine.input_at(at("10:31:10"));
    h.tick(at("10:31:11"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:31:10")));
    assert!(
        h.state.snapshot().away_prompt.is_none(),
        "a break is not time to ask about"
    );
}

#[test]
fn a_break_that_runs_out_during_a_sleep_resumes_at_the_wake_not_inside_the_sleep() {
    let mut h = tracking();
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: Some(Duration::from_secs(15 * 60)),
            at: at("10:05:00"),
        },
    );
    h.system(SystemEvent::Suspending, at("10:10:00"));
    h.machine.sleep_until(at("13:00:00"));
    h.worker.on_system(SystemNotice {
        event: SystemEvent::Resumed,
        at: at("13:00:00"),
        done: None,
    });
    // The key press that woke it is recent: someone is there.
    h.machine.input_at(at("13:00:01"));
    h.tick(at("13:00:02"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(
        h.tracking_since(),
        Some(at("13:00:00")),
        "from the wake, not from the end of the break"
    );
    h.machine.input_at(at("13:10:30"));
    h.tick(at("13:10:31"));
    assert!(
        h.queued()
            .iter()
            .skip(1)
            .all(|block| block.started_at >= at("13:00:00")),
        "{:?}",
        h.spans()
    );
}

#[test]
fn a_break_that_runs_out_during_a_sleep_nobody_announced_resumes_after_it() {
    let mut h = tracking();
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: Some(Duration::from_secs(15 * 60)),
            at: at("10:05:00"),
        },
    );
    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    h.machine.sleep_until(at("13:00:00"));
    h.machine.input_at(at("13:00:01"));
    h.machine.set(at("13:00:02"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.tracking_since(), Some(at("13:00:02")));
}

#[test]
fn time_away_can_be_kept_after_the_sign_in_expired() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    h.machine.input_at(at("10:40:00"));
    h.tick(at("10:40:01"));
    assert!(h.state.snapshot().away_prompt.is_some());
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::SignInExpired {
        owner,
        reason: "The refresh token is no longer valid.".into(),
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert!(
        h.state.snapshot().away_prompt.is_some(),
        "the question stays"
    );
    h.command(at("10:40:05"), Command::ResolveAway { keep: true });
    let kept: Vec<Activity> = h
        .queued()
        .into_iter()
        .filter(|a| a.end_reason == Some(EndReason::Idle) && a.started_at >= at("10:02:00"))
        .collect();
    assert_eq!(kept.first().map(|a| a.started_at), Some(at("10:02:00")));
    assert_eq!(kept.last().map(|a| a.ended_at), Some(at("10:40:00")));
}

#[test]
fn upload_events_of_the_previous_account_are_ignored() {
    let mut h = tracking();
    let other = owner_key("bob@acme.com");
    h.worker.on_upload(UploadEvent::WorkspaceLost {
        owner: other.clone(),
        workspace_id: wid("w1"),
    });
    h.worker.on_upload(UploadEvent::DeviceRevoked {
        owner: other.clone(),
        workspace_id: wid("w1"),
    });
    h.worker.on_upload(UploadEvent::SignInExpired {
        owner: other,
        reason: "refused".into(),
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(
        h.state
            .snapshot()
            .projects
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        [pid("p1")]
    );
}

#[test]
fn going_idle_ends_the_block_at_the_last_input_and_asks_about_the_time_away() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:06:00"));
    assert_eq!(
        h.status(),
        TrackerStatus::Tracking,
        "not idle for five minutes yet"
    );
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert_eq!(h.state.snapshot().away_since, Some(at("10:02:00")));
    let queued = h.queued();
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:02:00"), Some(EndReason::Idle))]
    );
    assert!(
        queued[0].screenshot_png_base64.is_none(),
        "no capture for an idle end"
    );
    assert!(h.heartbeats().contains(&PresenceStatus::Idle));

    // Back at 10:30: a new block starts there, and the question is asked.
    h.machine.input_at(at("10:30:00"));
    h.tick(at("10:30:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:30:00")));
    let prompt = h.state.snapshot().away_prompt.expect("asked");
    assert_eq!(
        (prompt.from, prompt.to, prompt.minutes()),
        (at("10:02:00"), at("10:30:00"), 28)
    );
    assert_eq!(prompt.project_title, "Project p1");

    // Kept: 28 minutes as idle blocks of at most one interval, with the note.
    h.command(
        at("10:30:05"),
        Command::SetTaskNote {
            note: "Client call".into(),
            at: at("10:30:05"),
        },
    );
    h.command(at("10:30:10"), Command::ResolveAway { keep: true });
    let kept: Vec<Activity> = h.queued().into_iter().skip(1).collect();
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0].started_at, at("10:02:00"));
    assert_eq!(kept[2].ended_at, at("10:30:00"));
    for block in &kept {
        let length = (block.ended_at - block.started_at).num_seconds();
        assert!(length <= 600);
        assert_eq!(block.productivity, Some(0));
        assert_eq!(block.idle_seconds, Some(length as u32));
        assert_eq!(block.end_reason, Some(EndReason::Idle));
        assert!(
            block.timeline.is_empty() && block.active_windows.is_empty() && !block.has_captures()
        );
        assert_eq!(
            (block.keyboard_hits, block.mouse_clicks),
            (None, None),
            "not measured while away"
        );
        assert_eq!(block.description.as_deref(), Some("Client call"));
        assert_eq!(block.project_id, pid("p1"));
    }
    assert!(h.state.snapshot().away_prompt.is_none());
}

#[test]
fn time_away_is_discarded_unless_kept() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    h.machine.input_at(at("10:40:00"));
    h.tick(at("10:40:01"));
    assert!(h.state.snapshot().away_prompt.is_some());
    h.command(at("10:40:05"), Command::ResolveAway { keep: false });
    assert_eq!(h.queued().len(), 1, "only the block before leaving");
    assert!(h.state.snapshot().away_prompt.is_none());
}

#[test]
fn an_idle_cut_lands_in_a_block_held_across_the_interval_boundary() {
    // Idle after 15 minutes; the user stops at 10:07.
    let mut h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 15)], None)),
        false,
        None,
    );
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:10:01"));
    assert!(
        h.queued().is_empty(),
        "the 10:00-10:10 block waits for the user"
    );
    assert_eq!(
        h.state.snapshot().unsaved_since,
        Some(at("10:00:00")),
        "and still counts in the live totals"
    );
    h.tick(at("10:22:00"));
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:07:00"), Some(EndReason::Idle))]
    );
}

#[test]
fn locking_ends_the_block_without_a_capture_and_unlocking_starts_a_new_one() {
    let mut h = tracking();
    // Locked before the block's screenshot moment (50%: 10:05).
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));
    h.system(SystemEvent::Locked, at("10:04:30"));
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:04:30"), Some(EndReason::Lock))]
    );
    assert!(
        h.queued()[0].screenshot_png_base64.is_none(),
        "never a capture of a locked screen"
    );
    assert_eq!(h.machine.captures(), 0);
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(h.heartbeats().contains(&PresenceStatus::Locked));

    // Locked for lunch: nothing is recorded, idle or not.
    h.tick(at("11:00:00"));
    assert_eq!(h.queued().len(), 1);
    h.system(SystemEvent::Unlocked, at("11:30:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("11:30:00")));
}

#[test]
fn a_disconnected_session_is_not_tracked_until_it_is_back_and_unlocked() {
    let mut h = tracking();
    h.machine.input_at(at("10:03:00"));
    h.system(SystemEvent::Locked, at("10:04:00"));
    h.system(SystemEvent::Disconnected, at("10:04:01"));
    // Back on the seat (VT switch back), but still on the lock screen.
    h.system(SystemEvent::Connected, at("10:30:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    h.system(SystemEvent::Unlocked, at("10:30:20"));
    assert_eq!(h.tracking_since(), Some(at("10:30:20")));
}

#[test]
fn the_block_is_saved_locally_before_the_machine_sleeps() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:30"));
    h.tick(at("10:05:30"));
    h.upload_commands();
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    h.machine.set(at("10:06:00"));
    h.worker.on_system(SystemNotice {
        event: SystemEvent::Suspending,
        at: at("10:06:00"),
        done: Some(done),
    });
    assert!(
        saved.try_recv().is_ok(),
        "logind is told the block is saved"
    );
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]
    );
    assert!(h.upload_commands().is_empty(), "no network before sleep");

    // Three hours later.
    h.machine.sleep_until(at("13:06:00"));
    h.worker.on_system(SystemNotice {
        event: SystemEvent::Resumed,
        at: at("13:06:00"),
        done: None,
    });
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(
        h.tracking_since(),
        Some(at("13:06:00")),
        "a new block from the wake"
    );
    assert_eq!(h.queued().len(), 1, "the sleep is not reported twice");
    assert!(h
        .upload_commands()
        .iter()
        .any(|c| matches!(c, UploadCommand::RowsQueued)));
}

#[test]
fn a_sleep_nobody_announced_ends_the_block_at_the_tick_before_it() {
    let mut h = tracking();
    h.machine.input_at(at("10:03:59"));
    h.tick(at("10:04:00"));
    // The lid closed; logind said nothing. Three hours on the wall clock,
    // two seconds of awake time.
    h.machine.sleep_until(at("13:04:00"));
    h.machine.set(at("13:04:02"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:04:00"), Some(EndReason::Sleep))]
    );
    assert_eq!(h.tracking_since(), Some(at("13:04:02")));
}

#[test]
fn a_sleep_that_never_came_does_not_stop_tracking_for_good() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.control_requests();
    h.system(SystemEvent::Suspending, at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(
        h.heartbeats().is_empty(),
        "nothing goes over the network before sleep"
    );
    // No resume was heard, but the user types: the machine is awake.
    h.machine.input_at(at("10:06:40"));
    h.tick(at("10:06:41"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:06:40")));
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]
    );
}

#[test]
fn the_session_ending_saves_the_block_and_a_cancelled_shutdown_goes_on() {
    let mut h = tracking();
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:07:00"));
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    h.machine.set(at("10:07:30"));
    h.worker.on_system(SystemNotice {
        event: SystemEvent::SessionEnding,
        at: at("10:07:30"),
        done: Some(done),
    });
    assert!(saved.try_recv().is_ok());
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:07:30"), Some(EndReason::Shutdown))]
    );
    // A second notice changes nothing.
    h.system(SystemEvent::SessionEnding, at("10:07:31"));
    assert_eq!(h.queued().len(), 1);

    h.system(SystemEvent::SessionContinues, at("10:08:00"));
    assert_eq!(h.tracking_since(), Some(at("10:08:00")));
}

#[test]
fn switching_project_ends_the_block_at_the_click() {
    let mut h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))),
        false,
        None,
    );
    h.machine.input_at(at("10:05:00"));
    h.command(
        at("10:06:01"),
        Command::SelectProject {
            project_id: pid("p2"),
            at: at("10:06:00"),
        },
    );
    let queued = h.queued();
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]
    );
    assert_eq!(
        queued[0].project_id,
        pid("p1"),
        "the work so far belongs to the project it was done on"
    );
    assert_eq!(h.state.snapshot().selected_project, Some(pid("p2")));
    assert_eq!(h.tracking_since(), Some(at("10:06:00")));
}

#[test]
fn a_project_that_disappears_stops_tracking_or_moves_to_the_only_one_left() {
    let mut h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))),
        false,
        None,
    );
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:06:00"));
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p2", 5)]));
    h.worker.publish();
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]
    );
    assert_eq!(
        h.state.snapshot().selected_project,
        Some(pid("p2")),
        "exactly one left"
    );
    assert!(h.notices().iter().any(|n| matches!(n,
        NoticeKind::ProjectUnavailable { title, now_tracking: Some(next) } if title == "Project p1" && next == "Project p2")));

    // Gone again, with two others left: choose.
    h.machine.set(at("10:20:00"));
    h.worker.on_control(ControlEvent::Projects(vec![
        project("p3", 5),
        project("p4", 5),
    ]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::ChooseProject);
    assert!(h.tracking_since().is_none());
}

#[test]
fn a_refreshed_policy_applies_from_the_next_block() {
    let mut h = tracking();
    let mut shorter = project("p1", 5);
    shorter.interval_minutes = 5;
    h.machine.input_at(at("10:01:00"));
    h.worker.on_control(ControlEvent::Projects(vec![shorter]));
    h.worker.publish();
    assert_eq!(h.state.snapshot().interval_minutes, 5);
    for minute in 2..=10 {
        h.machine.input_at(at(&format!("10:{minute:02}:00")));
        h.tick(at(&format!("10:{minute:02}:00")));
    }
    // The block in progress kept its ten minutes.
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:10:00"), Some(EndReason::Interval))]
    );
    assert_eq!(h.state.snapshot().next_capture_at, Some(at("10:15:00")));
}

#[test]
fn a_new_task_note_ends_the_block_or_relabels_a_young_one() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:30"));
    h.command(
        at("10:00:30"),
        Command::SetTaskNote {
            note: "Invoices".into(),
            at: at("10:00:30"),
        },
    );
    assert!(
        h.queued().is_empty(),
        "a block under a minute takes the note"
    );
    assert_eq!(h.state.snapshot().task_note, "Invoices");

    h.machine.input_at(at("10:05:00"));
    h.command(
        at("10:05:00"),
        Command::SetTaskNote {
            note: "Client call".into(),
            at: at("10:05:00"),
        },
    );
    let queued = h.queued();
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:05:00"), Some(EndReason::Task))]
    );
    assert_eq!(queued[0].description.as_deref(), Some("Invoices"));
    assert_eq!(h.tracking_since(), Some(at("10:00:00")), "tracking goes on");
    assert_eq!(h.state.snapshot().block_started_at, Some(at("10:05:00")));

    // The same note again is no change.
    h.command(
        at("10:06:00"),
        Command::SetTaskNote {
            note: " Client  call ".into(),
            at: at("10:06:00"),
        },
    );
    assert_eq!(h.queued().len(), 1);
}

#[test]
fn sync_now_saves_a_block_over_a_minute_old_and_retries_uploads() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:20"));
    h.command(at("10:00:30"), Command::SyncNow { at: at("10:00:30") });
    assert!(h.queued().is_empty(), "under a minute: keeps running");
    assert!(h
        .upload_commands()
        .iter()
        .any(|c| matches!(c, UploadCommand::Now)));

    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:00"), Command::SyncNow { at: at("10:04:00") });
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:04:00"), Some(EndReason::Interval))]
    );
    assert_eq!(
        h.state.snapshot().next_capture_at,
        Some(at("10:14:00")),
        "a full interval from the cut"
    );
}

#[test]
fn a_screenshot_is_taken_at_the_random_moment_or_at_an_early_end() {
    let mut h = tracking();
    // 50%: 10:05.
    h.machine.input_at(at("10:04:59"));
    h.tick(at("10:04:59"));
    assert_eq!(h.machine.captures(), 0);
    h.tick(at("10:05:01"));
    assert_eq!(h.machine.captures(), 1);
    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    let queued = h.queued();
    assert_eq!(queued[0].screenshot_captured_at, Some(at("10:05:01")));
    assert!(
        queued[0].screenshot_png_base64.is_some()
            && queued[0].screenshot_thumb_jpg_base64.is_some()
    );
    assert_eq!(h.machine.captures(), 1, "one per block");
    let preview = h
        .state
        .snapshot()
        .last_screenshot
        .expect("shown in the clients");
    assert_eq!(
        preview.taken_at,
        at("10:05:01"),
        "when it was taken, not when the block was saved"
    );

    // Paused at 10:12, before the next block's moment (10:15): taken then.
    h.machine.input_at(at("10:12:00"));
    h.command(
        at("10:12:00"),
        Command::Pause {
            resume_after: None,
            at: at("10:12:00"),
        },
    );
    assert_eq!(h.machine.captures(), 2);
    assert_eq!(h.queued()[1].screenshot_captured_at, Some(at("10:12:00")));
}

#[test]
fn a_refused_sign_in_stops_tracking_keeps_the_queue_and_asks_for_the_password() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.machine.set(at("10:05:00"));
    h.control_requests();
    h.upload_commands();
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::SignInExpired {
        owner,
        reason: "The refresh token is no longer valid.".into(),
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:05:00"), Some(EndReason::SignOut))]
    );
    let snapshot = h.state.snapshot();
    assert!(snapshot
        .login_error
        .unwrap()
        .contains("The refresh token is no longer valid."));
    assert_eq!(
        (snapshot.display_name.as_str(), snapshot.workspace.as_str()),
        ("alice@acme.com", "Acme"),
        "prefilled"
    );
    assert!(h
        .upload_commands()
        .iter()
        .any(|c| matches!(c, UploadCommand::Account(None))));
    assert!(h
        .control_requests()
        .iter()
        .any(|r| matches!(r, ControlRequest::UseAccount(None))));
    assert!(h.heartbeats().is_empty(), "no heartbeats without a sign-in");

    // The password, entered again.
    h.command(
        at("10:10:00"),
        Command::SignIn {
            workspace: None,
            username: "alice@acme.com".into(),
            password: Secret::new("new"),
            server: None,
        },
    );
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, SERVER);
    assert_eq!(account.device_id.as_deref(), Some(DEVICE_ID));
    // The saved workspace is picked again among several.
    let workspaces = vec![workspace("w0", "Other"), workspace("w1", "Acme")];
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(workspaces),
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:10:00")));
    let secrets = h
        .credentials
        .load_account("alice@acme.com")
        .unwrap()
        .unwrap();
    assert_eq!(
        secrets.password.as_deref(),
        Some("new"),
        "remembered in the keyring"
    );
    assert_eq!(
        secrets.device_signing_key,
        Some(KEY.to_vec()),
        "the signing key is kept"
    );
    assert_eq!(
        h.queued().len(),
        1,
        "the earlier block is still queued for this account"
    );
}

#[test]
fn a_sign_in_with_a_server_address_saves_it() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    h.command(
        at("10:00:00"),
        Command::SignIn {
            workspace: None,
            username: "alice@acme.com".into(),
            password: Secret::new("pw"),
            server: Some("ftp://nope".into()),
        },
    );
    assert!(
        h.state.snapshot().login_error.unwrap().contains("https://"),
        "an unusable address is refused at once"
    );
    assert!(h.control_requests().is_empty());

    h.command(
        at("10:00:05"),
        Command::SignIn {
            workspace: None,
            username: "alice@acme.com".into(),
            password: Secret::new("pw"),
            server: Some("tracker.acme.com/".into()),
        },
    );
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, "https://tracker.acme.com");
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme")]),
    });
    h.worker.on_control(ControlEvent::Projects(vec![
        project("a", 5),
        project("b", 5),
    ]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(
        snapshot.status,
        TrackerStatus::ChooseProject,
        "a first sign-in with several projects asks"
    );
    assert_eq!(snapshot.server_url, "https://tracker.acme.com");
    assert_eq!(
        h.worker.saved.as_ref().unwrap().server.as_deref(),
        Some("https://tracker.acme.com")
    );
}

#[test]
fn signing_out_saves_the_block_says_goodbye_and_forgets_the_account() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:31"), Command::SignOut { at: at("10:04:30") });
    let farewell = h.farewells.try_recv().expect("a last heartbeat");
    assert_eq!(
        farewell.heartbeat.map(|hb| hb.status),
        Some(PresenceStatus::SignedOut)
    );
    assert!(h
        .upload_commands()
        .iter()
        .any(|c| matches!(c, UploadCommand::SignOut { .. })));
    // Saved for the account, which uploads it when it signs in again.
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:04:30"), Some(EndReason::SignOut))]
    );
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert!(
        snapshot.display_name.is_empty()
            && snapshot.projects.is_empty()
            && snapshot.recent_blocks.is_empty()
    );
    assert_eq!(
        snapshot.pending_uploads, 0,
        "the next account does not see this one's queue"
    );
    assert!(
        h.credentials
            .load_account("alice@acme.com")
            .unwrap()
            .is_none(),
        "the keyring item is gone"
    );
}

#[tokio::test]
async fn quitting_saves_the_block_at_the_click_and_says_stopped() {
    let mut h = tracking();
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:07:01"));
    let mut farewells =
        std::mem::replace(&mut h.farewells, tokio::sync::mpsc::unbounded_channel().1);
    let answered = tokio::spawn(async move {
        let farewell = farewells.recv().await.expect("a last heartbeat");
        let status = farewell.heartbeat.as_ref().map(|hb| hb.status);
        let _ = farewell.done.send(());
        status
    });
    h.worker.quit(at("10:07:00")).await;
    assert_eq!(answered.await.unwrap(), Some(PresenceStatus::Stopped));
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:07:00"), Some(EndReason::Quit))]
    );
    assert!(h
        .upload_commands()
        .iter()
        .any(|c| matches!(c, UploadCommand::Shutdown { .. })));
}

#[test]
fn heartbeats_carry_the_device_project_and_app_and_follow_each_change() {
    let mut h = tracking();
    let requests = h.control_requests();
    let heartbeat = requests
        .iter()
        .find_map(|r| match r {
            ControlRequest::Heartbeat(hb) => Some(hb),
            _ => None,
        })
        .expect("sent at once");
    assert_eq!(heartbeat.status, PresenceStatus::Tracking);
    assert_eq!(heartbeat.device_id, DEVICE_ID);
    assert_eq!(heartbeat.project_id, Some(pid("p1")));
    assert_eq!(
        heartbeat.current_app.as_deref(),
        Some("code"),
        "the session records the active window"
    );
    assert_eq!(heartbeat.agent_version.as_deref(), Some("9.9.9"));
    assert_eq!(heartbeat.platform.as_deref(), Some("linux"));

    // No repeat within the minute while nothing changes...
    h.machine.input_at(at("10:00:30"));
    h.tick(at("10:00:30"));
    assert!(h.heartbeats().is_empty());
    // ...but at once on a change, without the app while paused.
    h.command(
        at("10:01:30"),
        Command::Pause {
            resume_after: None,
            at: at("10:01:30"),
        },
    );
    let paused: Vec<Heartbeat> = h
        .control_requests()
        .into_iter()
        .filter_map(|r| match r {
            ControlRequest::Heartbeat(hb) => Some(*hb),
            _ => None,
        })
        .collect();
    assert_eq!(paused.len(), 1);
    assert_eq!(paused[0].status, PresenceStatus::Paused);
    assert_eq!(paused[0].current_app, None);
    assert_eq!(paused[0].status_since, Some(at("10:01:30")));
}

#[test]
fn an_autostart_that_starts_tracking_says_so() {
    let h = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], None)),
        true,
        None,
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.notices().iter().any(|n| matches!(n,
        NoticeKind::TrackingStartedAutomatically { project_title } if project_title == "Project p1")));
    let manual = harness_at(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], None)),
        false,
        None,
    );
    assert!(manual.notices().is_empty());
}

#[test]
fn starting_a_stretch_resets_the_clocks_and_clears_a_pause() {
    let mut h = tracking();
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: Some(Duration::from_secs(30 * 60)),
            at: at("10:05:00"),
        },
    );
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:35:00")));
    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    let s = h.state.snapshot();
    assert_eq!(s.status, TrackerStatus::Tracking);
    assert!(s.paused_since.is_none() && s.resume_at.is_none());
    assert_eq!(
        (s.tracking_since, s.block_started_at),
        (Some(at("10:10:00")), Some(at("10:10:00")))
    );
    assert_eq!(
        s.next_capture_at,
        Some(at("10:20:00")),
        "a full interval until the block is saved"
    );
}

// ---- Choosing the workspace -----------------------------------------------------------

fn workspace(name: &str, title: &str) -> Workspace {
    Workspace {
        id: wid(name),
        name: title.into(),
        role: Some("Member".into()),
        plan_name: None,
        is_owner: false,
        is_archived: false,
    }
}

/// Type the email and password; the server accepts them.
fn typed_sign_in(h: &mut Harness, workspace: Option<&str>) -> Account {
    h.command(
        at("10:00:00"),
        Command::SignIn {
            workspace: workspace.map(Into::into),
            username: "alice@acme.com".into(),
            password: Secret::new("pw"),
            server: None,
        },
    );
    assert!(h.state.snapshot().signing_in);
    match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => {
            assert_eq!(
                (
                    account.username.as_str(),
                    account.password.expose(),
                    account.workspace_id.as_deref()
                ),
                ("alice@acme.com", "pw", None)
            );
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

#[test]
fn no_workspace_asks_to_create_one_or_be_invited() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![]),
    });
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.login_error.as_deref(), Some(NO_WORKSPACE));
    assert!(!snapshot.signing_in && snapshot.workspace_choices.is_empty());
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert!(h.worker.saved.is_none());
}

#[test]
fn one_workspace_is_used_at_once_and_remembered() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme")]),
    });
    let used = used_account(&mut h);
    assert_eq!(
        (used.workspace_id.clone(), used.workspace_name.as_str()),
        (Some(wid("w1")), "Acme")
    );
    assert!(
        used.password.is_empty(),
        "the client kept never holds the typed password"
    );
    assert!(
        used.signing_key.is_some(),
        "a device signing key is made at the first sign-in"
    );
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!(
        saved.workspace,
        Some(SavedWorkspace {
            id: wid("w1"),
            name: "Acme".into()
        })
    );
    assert_eq!(h.state.snapshot().workspace, "Acme");
    h.machine.input_at(at("10:04:30"));
    h.tick(at("10:05:00"));
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: None,
            at: at("10:05:00"),
        },
    );
    assert_eq!(
        h.rows()[0].0,
        wid("w1"),
        "blocks are queued in their workspace"
    );
}

#[test]
fn several_workspaces_are_offered_with_their_roles_and_the_chosen_one_is_used() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    let mut globex = workspace("w2", "Globex");
    globex.role = Some("Owner".into());
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme"), globex]),
    });
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.workspace_choices.len(), 2);
    assert_eq!(snapshot.workspace_choices[1].role.as_deref(), Some("Owner"));
    assert!(!snapshot.signing_in && snapshot.status == TrackerStatus::SignedOut);
    assert!(
        h.worker.saved.is_none(),
        "nothing saved before a workspace is chosen"
    );

    h.command(at("10:00:10"), Command::ChooseWorkspace { index: 1 });
    assert_eq!(used_account(&mut h).workspace_id, Some(wid("w2")));
    assert!(h.state.snapshot().workspace_choices.is_empty());
    assert_eq!(
        h.worker
            .saved
            .as_ref()
            .unwrap()
            .workspace
            .as_ref()
            .map(|w| w.id.clone()),
        Some(wid("w2"))
    );
}

#[test]
fn a_named_workspace_is_picked_without_asking() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("globex"));
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]),
    });
    assert_eq!(used_account(&mut h).workspace_id, Some(wid("w2")));

    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some(&wid("w1")));
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]),
    });
    assert_eq!(
        used_account(&mut h).workspace_id,
        Some(wid("w1")),
        "by id too"
    );
}

#[test]
fn going_back_from_the_picker_forgets_the_sign_in() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("a", "A"), workspace("b", "B")]),
    });
    h.command(at("10:00:05"), Command::CancelWorkspaceChoice);
    assert!(h.worker.pending.is_none() && h.worker.choices.is_empty());
    assert!(h.state.snapshot().workspace_choices.is_empty());
    h.command(at("10:00:06"), Command::ChooseWorkspace { index: 0 });
    assert!(h.control_requests().is_empty());
}

#[test]
fn a_saved_sign_in_without_a_workspace_must_sign_in_again() {
    let mut old = saved(vec![project("p1", 5)], None);
    old.session.workspace = None;
    let h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
}

fn list_workspaces(h: &mut Harness) {
    h.control_requests();
    h.upload_commands();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    assert!(h.state.snapshot().finding_workspaces);
    assert!(matches!(
        h.control_requests().pop(),
        Some(ControlRequest::MyWorkspaces)
    ));
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![
        workspace("w1", "Acme"),
        workspace("w2", "Globex"),
    ])));
    let snapshot = h.state.snapshot();
    assert!(snapshot.workspace_choices[0].is_current && !snapshot.workspace_choices[1].is_current);
}

#[test]
fn switching_workspace_saves_the_block_and_keeps_the_sign_in() {
    let mut h = tracking();
    let tokens = h.worker.account.as_ref().unwrap().tokens.clone();
    list_workspaces(&mut h);
    h.machine.input_at(at("10:01:59"));
    h.command(
        at("10:02:01"),
        Command::SwitchToWorkspace {
            index: 1,
            at: at("10:02:00"),
        },
    );
    assert!(h.farewells.try_recv().is_err(), "no sign-out");
    let rows = h.rows();
    assert_eq!(
        (rows[0].1.ended_at, rows[0].0.clone()),
        (at("10:02:00"), wid("w1")),
        "queued in the old workspace"
    );

    let requests = h.control_requests();
    assert!(
        !requests
            .iter()
            .any(|r| matches!(r, ControlRequest::SignIn(_))),
        "no new token, no password"
    );
    let used = requests.iter().rev().find_map(|r| match r {
        ControlRequest::UseAccount(Some(account)) => Some(account.clone()),
        _ => None,
    });
    let used = used.expect("the new workspace is used");
    assert_eq!(used.workspace_id, Some(wid("w2")));
    assert!(used.tokens.same_cell(&tokens), "the same token");
    assert!(requests
        .iter()
        .any(|r| matches!(r, ControlRequest::RefreshProjects)));
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ControlRequest::DeviceStatus)),
        "checked in the new workspace"
    );
    assert!(h.upload_commands().iter().any(
        |c| matches!(c, UploadCommand::Account(Some(a)) if a.workspace_id == Some(wid("w2")))
    ));

    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!(
        saved.workspace,
        Some(SavedWorkspace {
            id: wid("w2"),
            name: "Globex".into()
        })
    );
    assert_eq!(saved.project_id, None);
    assert!(saved.projects.is_none());
    let snapshot = h.state.snapshot();
    assert_eq!(
        (snapshot.status, snapshot.workspace.as_str()),
        (TrackerStatus::Connecting, "Globex")
    );
    assert!(snapshot.projects.is_empty() && snapshot.workspace_choices.is_empty());

    h.worker.on_control(ControlEvent::Projects(vec![
        project("p7", 5),
        project("p8", 5),
    ]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::ChooseProject);
}

#[test]
fn switching_with_no_other_workspace_says_so() {
    let mut h = tracking();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    h.worker
        .on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace(
            "w1", "Acme",
        )])));
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.workspace_note.as_deref(), Some(ONE_WORKSPACE));
    assert!(!snapshot.finding_workspaces && snapshot.workspace_choices.is_empty());
    assert_eq!(h.status(), TrackerStatus::Tracking, "still signed in");
}

#[test]
fn losing_access_to_the_workspace_stops_tracking_and_asks_for_another() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.control_requests();
    h.worker.on_upload(UploadEvent::WorkspaceLost {
        owner: h.owner(),
        workspace_id: wid("other"),
    });
    h.worker.publish();
    assert_eq!(
        h.status(),
        TrackerStatus::Tracking,
        "rows of another workspace: not about this one"
    );

    h.worker.on_control(ControlEvent::WorkspaceLost(wid("w1")));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::ChooseProject);
    assert!(snapshot.tracking_since.is_none());
    assert_eq!(snapshot.workspace_lost.as_deref(), Some("Acme"));
    assert_eq!(
        snapshot.workspace_note.as_deref(),
        Some("You no longer have access to Acme.")
    );
    assert!(snapshot
        .notices
        .iter()
        .any(|n| matches!(&n.kind, NoticeKind::WorkspaceLost { name } if name == "Acme")));
    assert!(snapshot.finding_workspaces);
    assert!(
        h.control_requests()
            .iter()
            .any(|r| matches!(r, ControlRequest::MyWorkspaces)),
        "the picker is opened"
    );
    let rows = h.rows();
    assert_eq!(
        (rows[0].1.ended_at, rows[0].0.clone()),
        (at("10:04:00"), wid("w1")),
        "the work is kept"
    );
    assert!(
        h.heartbeats().is_empty(),
        "no heartbeat without a workspace"
    );

    // Another workspace is chosen: tracking can start there.
    h.worker
        .on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace(
            "w2", "Globex",
        )])));
    h.command(
        at("10:05:00"),
        Command::SwitchToWorkspace {
            index: 0,
            at: at("10:05:00"),
        },
    );
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p9", 5)]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(
        (
            snapshot.status,
            snapshot.workspace.as_str(),
            snapshot.workspace_lost.as_deref()
        ),
        (TrackerStatus::Tracking, "Globex", None)
    );
}

#[test]
fn tracking_active_follows_pause_and_resume_lifecycle() {
    let h_unsigned = harness_at(at("10:00:00"), None, false, None);
    assert!(
        !h_unsigned.machine.is_tracking_active(),
        "sensors must start inactive before sign-in"
    );
    assert!(!h_unsigned.machine.is_input_running());
    assert!(!h_unsigned.machine.is_usage_running());

    let mut h = tracking();
    assert!(
        h.machine.is_tracking_active(),
        "sensors must activate when tracking a project"
    );
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 1);
    assert_eq!(h.machine.usage_starts(), 1);

    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: None,
            at: at("10:05:00"),
        },
    );
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must deactivate immediately when paused"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_stops(), 1);
    assert_eq!(h.machine.usage_stops(), 1);

    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    assert!(
        h.machine.is_tracking_active(),
        "sensors must reactivate upon resume"
    );
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 2);
    assert_eq!(h.machine.usage_starts(), 2);

    h.command(at("10:15:00"), Command::SignOut { at: at("10:15:00") });
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must deactivate upon sign-out"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_stops(), 2);
    assert_eq!(h.machine.usage_stops(), 2);
}

#[test]
fn sensor_lifecycle_stops_on_lock_and_resumes_on_unlock() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    h.worker.on_system(SystemNotice {
        event: SystemEvent::Locked,
        at: at("10:05:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must stop when the session is locked"
    );
    assert!(
        !h.machine.is_input_running(),
        "the listener must stop on lock"
    );
    assert!(
        !h.machine.is_usage_running(),
        "the window sampler must stop on lock"
    );

    h.worker.on_system(SystemNotice {
        event: SystemEvent::Unlocked,
        at: at("10:06:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(
        h.machine.is_tracking_active(),
        "sensors must resume when the session unlocks"
    );
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_away_and_resumes_on_return() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    // Past the idle threshold (5 min).
    h.machine.set(at("10:15:00"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must stop while the user is away"
    );
    assert!(
        !h.machine.is_input_running(),
        "the listener must stop when away"
    );
    assert!(
        !h.machine.is_usage_running(),
        "the window sampler must stop when away"
    );

    // The user comes back.
    h.machine.input_at(at("10:16:00"));
    h.machine.set(at("10:16:01"));
    h.worker.on_tick();
    h.worker.publish();
    assert!(
        h.machine.is_tracking_active(),
        "sensors reactivate upon return"
    );
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_workspace_lost() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    h.worker.on_control(ControlEvent::WorkspaceLost(wid("w1")));
    h.worker.publish();
    assert_ne!(h.status(), TrackerStatus::Tracking);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must stop when workspace access is lost"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
}

#[tokio::test]
async fn sensor_lifecycle_stops_on_quit_before_upload_grace() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    let machine = h.machine.clone();
    let quit_task = tokio::spawn(async move {
        h.worker.quit(at("10:05:00")).await;
        h
    });

    // While quit awaits the farewell and the upload grace, the sensors are
    // already halted.
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert!(
        !machine.is_tracking_active(),
        "sensors must stop BEFORE waiting for farewell/upload grace"
    );
    assert!(
        !machine.is_input_running(),
        "the listener must stop BEFORE waiting"
    );
    assert!(
        !machine.is_usage_running(),
        "the sampler must stop BEFORE waiting"
    );
    assert_eq!(machine.input_stops(), 1);
    assert_eq!(machine.usage_stops(), 1);

    let h = quit_task.await.unwrap();
    assert!(!h.machine.is_tracking_active());
}

#[test]
fn worker_fails_closed_when_consent_is_absent() {
    let h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], None)),
        LocalConsent::Absent,
    );
    assert!(
        !h.machine.is_tracking_active(),
        "absent consent must fail closed"
    );
    assert!(
        !h.machine.is_input_running(),
        "the listener must not start without consent"
    );
    assert!(
        !h.machine.is_usage_running(),
        "the sampler must not start without consent"
    );
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn sensor_lifecycle_per_sensor_denied_policy() {
    let h = tracking();
    let perms = ConsentedPermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        screenshot: false,
        webcam: false,
        running_programs: false,
        blur_screenshots: false,
    };
    h.machine.apply_sensor_policy(true, Some(&perms));
    assert!(
        h.machine.is_input_running(),
        "the listener runs for the keyboard"
    );
    assert!(
        !h.machine.is_usage_running(),
        "the sampler stays stopped without the active window"
    );
}

#[test]
fn sensor_lifecycle_handles_repeated_transitions_accurately() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert_eq!(h.machine.input_starts(), 1);
    assert_eq!(h.machine.usage_starts(), 1);

    for i in 1..=5 {
        let t_pause = at(&format!("10:{:02}:00", i * 2));
        let t_resume = at(&format!("10:{:02}:00", i * 2 + 1));

        h.command(
            t_pause,
            Command::Pause {
                resume_after: None,
                at: t_pause,
            },
        );
        assert!(!h.machine.is_tracking_active());
        assert!(!h.machine.is_input_running());
        assert!(!h.machine.is_usage_running());
        assert_eq!(h.machine.input_stops(), i);
        assert_eq!(h.machine.usage_stops(), i);

        h.command(t_resume, Command::Resume { at: t_resume });
        assert!(h.machine.is_tracking_active());
        assert!(h.machine.is_input_running());
        assert!(h.machine.is_usage_running());
        assert_eq!(h.machine.input_starts(), i + 1);
        assert_eq!(h.machine.usage_starts(), i + 1);
    }
}

// ---- Local consent UX and lifecycle integration tests ---------------------------------

#[test]
fn consent_1_fresh_config_absent() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(h.tracking_since().is_none());
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);

    h.tick(at("10:01:00"));
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_2_missing_consent_file_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("dosi-tracker").join("consent.json");
    assert_eq!(
        ConsentStore::load(&nonexistent).unwrap(),
        LocalConsent::Absent,
        "a missing file fails closed"
    );
    let (manager, error) = ConsentManager::load_or_fail_closed(&nonexistent);
    assert!(error.is_none());
    assert_eq!(manager.current(), LocalConsent::Absent);

    let h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        manager.current(),
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_3_explicit_granular_grant() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);

    let granular = ConsentedPermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        running_programs: false,
        screenshot: false,
        webcam: false,
        blur_screenshots: false,
    };
    h.command(
        at("10:00:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(granular),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(
        h.machine.is_input_running(),
        "the listener runs for the keyboard"
    );
    assert!(h.machine.keyboard());
    assert!(!h.machine.mouse(), "mouse not consented");
    assert!(!h.machine.is_usage_running(), "active window not consented");
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_4_decline_prevents_tracking() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    h.command(
        at("10:00:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    h.command(
        at("10:01:00"),
        Command::SelectProject {
            project_id: pid("p1"),
            at: at("10:01:00"),
        },
    );
    h.command(at("10:01:30"), Command::Resume { at: at("10:01:30") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_5_revoke_while_active() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    h.command(
        at("10:02:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(h.machine.input_stops() >= 1);
    assert!(h.machine.usage_stops() >= 1);
    assert_eq!(h.consent_store.current(), LocalConsent::Revoked, "saved");
    assert_eq!(
        h.lifecycle.lock().unwrap().consent(),
        LocalConsent::Revoked,
        "the gate's consent too"
    );
}

#[test]
fn consent_6_policy_narrowing() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Narrowing by user consent (uncheck active_window).
    let mut user_narrowed = defaults();
    user_narrowed.active_window = false;
    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(user_narrowed),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(
        !h.machine.is_usage_running(),
        "usage stopped after the user narrowed consent"
    );
    assert_eq!(h.machine.usage_stops(), 1);

    // Narrowing by the project's policy (keyboard and mouse disallowed).
    let restricted: Project = serde_json::from_value(serde_json::json!({
        "id": pid("p1"), "title": "Project p1", "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false,
        "allowKeyboard": false, "allowMouse": false,
        "allowActiveWindow": false, "allowRunningPrograms": false,
        "blurScreenshots": true,
    }))
    .unwrap();
    h.worker
        .on_control(ControlEvent::Projects(vec![restricted]));
    h.worker.publish();
    assert!(
        !h.machine.is_input_running(),
        "input stopped when the workspace policy disallowed it"
    );
    assert!(h.machine.input_stops() >= 1);
}

#[test]
fn consent_7_project_workspace_logout_transition() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // Signing out stops the sensors at once.
    h.command(at("10:05:00"), Command::SignOut { at: at("10:05:00") });
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());

    // Signed in with consent revoked: nothing starts.
    let h_revoked = harness_with_consent(
        at("10:10:00"),
        Some(saved(vec![project("p2", 5)], Some("p2"))),
        LocalConsent::Revoked,
    );
    assert_eq!(h_revoked.status(), TrackerStatus::ConsentRequired);
    assert!(!h_revoked.machine.is_tracking_active());
    assert_eq!(h_revoked.machine.input_starts(), 0);
    assert_eq!(h_revoked.machine.usage_starts(), 0);
}

#[test]
fn consent_8_persisted_explicit_choice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dosi-tracker").join("consent.json");
    let grant = LocalConsent::Granted(ConsentedPermissions {
        screenshot: true,
        webcam: false,
        keyboard: true,
        mouse: false,
        active_window: true,
        running_programs: false,
        blur_screenshots: true,
    });
    ConsentStore::save(&path, &grant).expect("save must succeed");
    assert_eq!(
        ConsentStore::load(&path).unwrap(),
        grant,
        "loaded consent must match the saved one exactly"
    );
}

#[test]
fn consent_9_corrupted_config_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("consent.json");
    for corrupt in [&b""[..], b"{ not valid json ::: ", b"{\"Granted\": 99999}"] {
        std::fs::write(&path, corrupt).unwrap();
        let (manager, error) = ConsentManager::load_or_fail_closed(&path);
        assert_eq!(manager.current(), LocalConsent::Absent, "{corrupt:?}");
        assert!(error.is_some(), "the damage is reported");
    }
}

#[test]
fn consent_10_pause_resume_quit() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    h.command(
        at("10:02:00"),
        Command::Pause {
            resume_after: None,
            at: at("10:02:00"),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert!(!h.machine.is_tracking_active());

    // Revoked while paused.
    h.command(
        at("10:02:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    // Resuming fails closed.
    h.command(at("10:03:00"), Command::Resume { at: at("10:03:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());

    // Granted again: tracking goes on.
    h.command(
        at("10:03:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(defaults()),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // A lock stops tracking.
    h.system(SystemEvent::Locked, at("10:04:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(!h.machine.is_tracking_active());

    // Revoked during the lock.
    h.command(
        at("10:04:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    h.system(SystemEvent::Unlocked, at("10:05:00"));
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must not restart after permission loss during lock"
    );
}

#[test]
fn consent_11_no_duplicate_thread_start_on_repeated_user_actions() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let initial_input_starts = h.machine.input_starts();
    let initial_usage_starts = h.machine.usage_starts();

    for i in 1..=5 {
        h.command(
            at(&format!("10:01:{i:02}")),
            Command::UpdateConsent {
                consent: LocalConsent::Granted(defaults()),
            },
        );
    }
    assert_eq!(
        h.machine.input_starts(),
        initial_input_starts,
        "no duplicate input starts on repeated grant"
    );
    assert_eq!(
        h.machine.usage_starts(),
        initial_usage_starts,
        "no duplicate usage starts on repeated grant"
    );

    for i in 1..=5 {
        h.command(
            at(&format!("10:02:{i:02}")),
            Command::Resume {
                at: at(&format!("10:02:{i:02}")),
            },
        );
        h.command(
            at(&format!("10:03:{i:02}")),
            Command::SelectProject {
                project_id: pid("p1"),
                at: at(&format!("10:03:{i:02}")),
            },
        );
    }
    assert_eq!(
        h.machine.input_starts(),
        initial_input_starts,
        "repeated commands do not duplicate threads"
    );
    assert_eq!(h.machine.usage_starts(), initial_usage_starts);
}

#[test]
fn consent_12_grant_persistence_failure_blocks_sensors_and_reports_error() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());

    h.consent_store.set_fail_saves(true);
    let grant = ConsentedPermissions {
        keyboard: true,
        mouse: true,
        screenshot: false,
        webcam: false,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(
        at("10:00:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(grant),
        },
    );

    // Granting must not start sensors until it is saved.
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must NOT start on a failed save"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);

    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_error.is_some(), "the failure is reported");
    assert!(snapshot.consent_unsynced);
    assert_eq!(
        snapshot.consent,
        LocalConsent::Absent,
        "the grant is not accepted on a failed save"
    );
}

#[test]
fn consent_13_revoke_persistence_failure_stops_sensors_and_reports_non_durable() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(defaults()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());

    h.consent_store.set_fail_saves(true);
    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );

    // A revocation stops the sensors at once, even when the save fails.
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "sensors must stop at once on revoke"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(h.machine.input_stops() >= 1);

    let snapshot = h.state.snapshot();
    let err_msg = snapshot
        .consent_error
        .as_ref()
        .expect("the failure is reported");
    assert!(
        err_msg.contains("non-durable across app restart"),
        "the restart limit is reported honestly"
    );
    assert!(snapshot.consent_unsynced);

    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "resume is blocked while unsynced"
    );
}

#[test]
fn consent_14_fake_worker_isolation_proves_no_disk_access() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(
        h.consent_store.load_count(),
        1,
        "loaded once from the memory store (by the harness, as the daemon does)"
    );
    assert_eq!(h.consent_store.save_count(), 0);

    let grant = ConsentedPermissions::default();
    h.command(
        at("10:00:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(grant),
        },
    );
    assert_eq!(
        h.consent_store.save_count(),
        1,
        "saved to the memory store on update"
    );

    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    assert_eq!(
        h.consent_store.save_count(),
        2,
        "the second save went to the memory store"
    );
    assert_eq!(
        h.consent_store.load_count(),
        1,
        "the worker never reads consent from disk"
    );
    assert_eq!(h.consent_store.current(), LocalConsent::Revoked);
}

#[test]
fn consent_15_partial_grant_omitted_capabilities_strictly_default_deny() {
    let json_only_mouse = r#"{"Granted": {"mouse": true}}"#;
    let consent: LocalConsent =
        serde_json::from_str(json_only_mouse).expect("partial json must deserialize");

    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    h.command(at("10:00:30"), Command::UpdateConsent { consent });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.mouse(), "mouse granted");
    assert!(!h.machine.keyboard(), "keyboard omitted -> denied");
    assert!(
        !h.machine.is_usage_running(),
        "active window omitted -> denied"
    );
    assert_eq!(h.machine.usage_starts(), 0);
}

fn broad() -> ConsentedPermissions {
    ConsentedPermissions {
        blur_screenshots: true,
        ..defaults()
    }
}

fn narrower() -> ConsentedPermissions {
    ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    }
}

#[test]
fn consent_16_narrowing_stops_withdrawn_sensors_before_blocked_save_completes() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.keyboard() && h.machine.mouse());
    assert!(h.machine.is_usage_running());
    assert!(h.machine.is_screenshot_allowed());

    // Inspect the machine WHILE the save is in flight.
    let machine = h.machine.clone();
    let hook_called = Arc::new(AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_barrier(move |_target| {
        hook_called_clone.store(true, Ordering::SeqCst);
        // Withdrawn permissions MUST be stopped before the save blocks.
        assert!(!machine.keyboard(), "keyboard MUST stop before the save");
        assert!(
            !machine.is_usage_running(),
            "usage sampler MUST stop before the save"
        );
        assert!(
            !machine.is_screenshot_allowed(),
            "screenshots MUST be off before the save"
        );
        // A retained permission stays.
        assert!(machine.mouse(), "mouse remains active");
        assert!(machine.is_input_running(), "input running for the mouse");
        assert_eq!(machine.captures(), 0, "no screen captures occurred");
        Ok(())
    });

    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(narrower()),
        },
    );
    assert!(
        hook_called.load(Ordering::SeqCst),
        "the hook ran during the save"
    );
    assert_eq!(h.consent_store.save_count(), 1);

    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.mouse());
    assert!(!h.machine.keyboard());
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());
}

#[test]
fn consent_17_mixed_narrowing_and_expansion_request_under_blocked_store() {
    // Mouse and keyboard, no screenshots.
    let initial = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: true,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(initial),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.keyboard());
    assert!(!h.machine.is_screenshot_allowed());

    // Withdraw the keyboard, add screenshots.
    let machine = h.machine.clone();
    let hook_called = Arc::new(AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_barrier(move |_target| {
        hook_called_clone.store(true, Ordering::SeqCst);
        assert!(
            !machine.keyboard(),
            "the withdrawn keyboard stopped before the save"
        );
        assert!(
            !machine.is_screenshot_allowed(),
            "an unsaved expansion must NOT be active"
        );
        assert!(machine.mouse(), "the retained mouse is active");
        Ok(())
    });
    let mixed = ConsentedPermissions {
        screenshot: true,
        keyboard: false,
        ..initial
    };
    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(mixed),
        },
    );

    assert!(hook_called.load(Ordering::SeqCst));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(
        h.machine.is_screenshot_allowed(),
        "screenshots start once the save succeeded"
    );
    assert!(!h.machine.keyboard());
    assert!(h.machine.mouse());
}

#[test]
fn consent_18_blocked_save_failure_fails_closed_and_prevents_resume() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    let machine = h.machine.clone();
    h.consent_store.set_save_barrier(move |_target| {
        // The withdrawn keyboard stopped before the write error.
        assert!(!machine.keyboard());
        Err(ConsentError::SaveFailed(
            "simulated disk I/O timeout during consent save".into(),
        ))
    });
    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(narrower()),
        },
    );

    // A failed save fails closed.
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "all sensors stopped on write failure"
    );
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());

    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_unsynced, "must report unsynced");
    assert_eq!(
        snapshot.consent,
        LocalConsent::Revoked,
        "the fail-closed state does not claim success"
    );
    let err = snapshot.consent_error.as_ref().unwrap();
    assert!(
        err.contains("non-durable across app restart"),
        "honestly reports the restart limit"
    );

    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(
        !h.machine.is_tracking_active(),
        "resume blocked while unsynced"
    );
}

#[test]
fn consent_19_multithreaded_barrier_proves_withdrawn_sensors_stopped_while_store_blocked() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    let enter_barrier = Arc::new(std::sync::Barrier::new(2));
    let exit_barrier = Arc::new(std::sync::Barrier::new(2));
    let enter_clone = enter_barrier.clone();
    let exit_clone = exit_barrier.clone();
    h.consent_store.set_save_barrier(move |_target| {
        enter_clone.wait(); // the save is now blocked
        exit_clone.wait(); // until the test thread checked the machine
        Ok(())
    });

    let machine = h.machine.clone();
    let worker_thread = std::thread::spawn(move || {
        h.command(
            at("10:01:00"),
            Command::UpdateConsent {
                consent: LocalConsent::Granted(narrower()),
            },
        );
        h
    });

    // The worker thread is blocked inside the save right now.
    enter_barrier.wait();
    assert!(
        !machine.keyboard(),
        "keyboard stopped while the store is blocked"
    );
    assert!(
        !machine.is_usage_running(),
        "usage stopped while the store is blocked"
    );
    assert!(
        !machine.is_screenshot_allowed(),
        "screenshots off while the store is blocked"
    );
    assert!(machine.mouse(), "the retained mouse still runs");
    assert_eq!(machine.captures(), 0, "no screen captures scheduled/taken");
    exit_barrier.wait();

    let h = worker_thread
        .join()
        .expect("the worker thread finishes cleanly");
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.consent_store.save_count(), 1);
}

#[test]
fn consent_20_withdrawn_sensors_stopped_before_reconcile_record_blocks() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // The worker blocks inside snapshot (record) until the test checked.
    let (enter_tx, enter_rx) = std::sync::mpsc::channel::<()>();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
    let exit_rx = Mutex::new(exit_rx);
    h.machine.set_on_snapshot(move || {
        let _ = enter_tx.send(());
        let _ = exit_rx.lock().unwrap().recv();
    });

    let machine = h.machine.clone();
    let worker_thread = std::thread::spawn(move || {
        h.command(
            at("10:01:00"),
            Command::UpdateConsent {
                consent: LocalConsent::Revoked,
            },
        );
        h
    });

    enter_rx
        .recv()
        .expect("the worker enters snapshot while finishing the block");
    // The save has not even started; the sensors are already stopped.
    assert!(
        !machine.is_tracking_active(),
        "tracking stopped before record/snapshot"
    );
    assert!(
        !machine.is_input_running(),
        "input stopped before record/snapshot"
    );
    assert!(
        !machine.is_usage_running(),
        "usage stopped before record/snapshot"
    );
    assert_eq!(
        machine.captures(),
        0,
        "no screen captures while record is blocked"
    );
    assert_eq!(
        machine.window_queries(),
        0,
        "no window queries while record is blocked"
    );
    assert_eq!(
        machine.process_queries(),
        0,
        "no process queries while record is blocked"
    );
    drop(exit_tx);

    let h = worker_thread
        .join()
        .expect("the worker thread finishes cleanly");
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.consent_store.save_count(), 1);
    assert_eq!(h.consent_store.current(), LocalConsent::Revoked);
    assert_eq!(
        h.machine.snapshot_calls(),
        1,
        "exactly one snapshot during finalization"
    );
    assert_eq!(h.machine.captures(), 0);
    assert_eq!(h.machine.window_queries(), 0);
    assert_eq!(h.machine.process_queries(), 0);
}

#[test]
fn consent_21_held_blocks_do_not_acquire_withdrawn_screen_at_finalization() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 30)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    h.machine.input_at(at("10:03:00"));

    // Past the block boundary with the last input at 10:03: under a 30-min
    // idle threshold the 10:00-10:10 block is held.
    h.tick(at("10:10:01"));
    assert_eq!(
        h.queued().len(),
        0,
        "the first block is held awaiting the user"
    );
    let captures_before_revoke = h.machine.captures();

    h.command(
        at("10:12:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Revoked,
        },
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(
        h.machine.snapshot_calls(),
        2,
        "snapshots for the held and the open block"
    );
    assert_eq!(
        h.machine.captures(),
        captures_before_revoke,
        "no new screen captures at finalization"
    );
    assert_eq!(
        h.machine.window_queries(),
        0,
        "no foreground window sampling after withdrawal"
    );
    assert_eq!(
        h.machine.process_queries(),
        0,
        "no running programs listing after withdrawal"
    );

    // The historical work is kept, without fresh capture data.
    let queued = h.queued();
    assert_eq!(
        queued.len(),
        2,
        "the held block and the closing sliver are finalized"
    );
    let held_block = &queued[0];
    assert_eq!(held_block.started_at, at("10:00:00"));
    assert_eq!(held_block.ended_at, at("10:10:00"));
    assert_eq!(
        held_block.keyboard_hits,
        Some(1),
        "authorized key presses of the held block are kept"
    );
    assert_eq!(
        held_block.mouse_clicks,
        Some(1),
        "authorized clicks of the held block are kept"
    );
    assert_eq!(
        held_block.active_windows.len(),
        1,
        "the authorized usage slice is kept"
    );
    assert_eq!(
        held_block.screenshot_png_base64, None,
        "no screenshot on the held block"
    );
    assert!(
        held_block.running_programs.is_empty(),
        "no process sampling on the held block"
    );

    let open_block = &queued[1];
    assert_eq!(open_block.started_at, at("10:10:00"));
    assert_eq!(open_block.ended_at, at("10:12:00"));
    assert_eq!(
        open_block.screenshot_png_base64, None,
        "no screenshot on the open block"
    );
    assert!(
        open_block.running_programs.is_empty(),
        "no process sampling on the open block"
    );
}

#[test]
fn consent_22_mixed_narrowing_stops_withdrawn_sensors_before_reconcile_record_blocks() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    h.machine.input_at(at("10:02:00"));

    h.command(
        at("10:05:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(narrower()),
        },
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(!h.machine.keyboard(), "keyboard stopped");
    assert!(!h.machine.is_usage_running(), "usage sampler stopped");
    assert!(!h.machine.is_screenshot_allowed(), "screenshots off");
    assert!(h.machine.mouse(), "the retained mouse still runs");

    h.tick(at("10:10:01"));
    assert_eq!(
        h.machine.captures(),
        0,
        "no screen captures at the block boundary"
    );
    assert_eq!(
        h.machine.window_queries(),
        0,
        "no foreground window sampling at the block boundary"
    );
    assert_eq!(
        h.machine.process_queries(),
        0,
        "no process queries at the block boundary"
    );

    let queued = h.queued();
    assert_eq!(queued.len(), 1, "the interval block is recorded");
    assert_eq!(
        queued[0].keyboard_hits, None,
        "the withdrawn keyboard is not counted (null, never 0)"
    );
    assert_eq!(
        queued[0].mouse_clicks,
        Some(1),
        "retained mouse clicks are kept"
    );
    assert_eq!(
        queued[0].screenshot_png_base64, None,
        "no screenshot attached"
    );
    assert!(
        queued[0].running_programs.is_empty(),
        "no running programs attached"
    );
}

// ---- Linux cases (design 5.2 and 6) ----------------------------------------------------

#[test]
fn manual_mode_never_starts_by_itself_and_needs_an_explicit_resume() {
    let machine = FakeMachine::new(at("10:00:00"));
    machine.set_manual();
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.fake = Some(machine);
    let mut h = build(s);
    let snapshot = h.state.snapshot();
    assert!(snapshot.manual_mode, "the banner shows");
    assert_eq!(
        snapshot.status,
        TrackerStatus::Paused,
        "never starts by itself"
    );
    assert!(!h.machine.is_tracking_active());

    h.command(at("10:01:00"), Command::Resume { at: at("10:01:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:01:00")));

    // Nothing tells that the user left: no idle cut, ever.
    h.tick(at("10:30:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // A lock still ends the block, and tracking does not resume by itself.
    h.system(SystemEvent::Locked, at("10:35:00"));
    h.system(SystemEvent::Unlocked, at("10:45:00"));
    assert_eq!(
        h.status(),
        TrackerStatus::Paused,
        "manual mode never auto-resumes"
    );
    assert_eq!(
        h.spans().last().map(|(_, end, reason)| (*end, *reason)),
        Some((at("10:35:00"), Some(EndReason::Lock)))
    );
    h.command(at("10:46:00"), Command::Resume { at: at("10:46:00") });
    assert_eq!(h.tracking_since(), Some(at("10:46:00")));
}

#[test]
fn manual_mode_pauses_after_the_maximum_hours_with_a_notice() {
    let machine = FakeMachine::new(at("10:00:00"));
    machine.set_manual();
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.fake = Some(machine);
    s.cfg.tracking.manual_max_hours = 2;
    let mut h = build(s);
    h.command(at("10:00:00"), Command::Resume { at: at("10:00:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    h.tick(at("11:59:58"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    h.tick(at("12:00:30"));
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(
        h.state.snapshot().paused_since,
        Some(at("12:00:00")),
        "the pause starts at the limit"
    );
    assert!(h
        .notices()
        .contains(&NoticeKind::ManualModePaused { hours: 2 }));
    assert_eq!(
        h.spans().last().map(|(_, end, _)| *end),
        Some(at("12:00:00")),
        "nothing is tracked past the limit"
    );
    assert!(
        h.worker.saved.as_ref().unwrap().pause.is_some(),
        "a real pause: it survives a restart"
    );
}

#[test]
fn a_timed_break_in_manual_mode_waits_for_the_user() {
    let machine = FakeMachine::new(at("10:00:00"));
    machine.set_manual();
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.fake = Some(machine);
    let mut h = build(s);
    h.command(at("10:00:00"), Command::Resume { at: at("10:00:00") });
    h.command(
        at("10:05:00"),
        Command::Pause {
            resume_after: Some(Duration::from_secs(600)),
            at: at("10:05:00"),
        },
    );
    h.tick(at("10:16:00"));
    assert_eq!(
        h.status(),
        TrackerStatus::Paused,
        "the break is over, but nothing tells the user is back"
    );
    h.command(at("10:17:00"), Command::Resume { at: at("10:17:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
}

#[test]
fn no_session_never_tracks_but_keeps_the_sign_in() {
    let machine = FakeMachine::new(at("10:00:00"));
    machine.set_session(SessionKind::Tty);
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.fake = Some(machine);
    let mut h = build(s);
    assert_eq!(h.status(), TrackerStatus::NoSession);
    assert!(!h.machine.is_tracking_active());
    h.command(at("10:01:00"), Command::Resume { at: at("10:01:00") });
    h.tick(at("10:20:00"));
    assert_eq!(h.status(), TrackerStatus::NoSession);
    assert!(h.queued().is_empty(), "nothing is recorded");
    assert!(
        h.heartbeats().contains(&PresenceStatus::Stopped),
        "presence says stopped"
    );
    assert!(
        h.upload_commands()
            .iter()
            .any(|c| matches!(c, UploadCommand::Account(Some(_)))),
        "the queue still uploads"
    );
    assert!(
        !h.state.snapshot().manual_mode,
        "no banner without a graphical session"
    );
}

#[test]
fn a_revoked_device_stops_tracking_and_heartbeats_and_parks_the_scope() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.control_requests();
    h.upload_commands();
    h.worker.on_control(ControlEvent::DeviceRevoked(wid("w1")));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::DeviceRevoked);
    assert!(!h.machine.is_tracking_active());
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:04:00"), Some(EndReason::SignOut))]
    );
    assert!(h.upload_commands().iter().any(|c| matches!(c,
        UploadCommand::ParkScope { workspace_id, reason } if *workspace_id == wid("w1") && reason == "device revoked")));
    assert!(h
        .notices()
        .iter()
        .any(|n| matches!(n, NoticeKind::DeviceRevoked { workspace } if workspace == "Acme")));
    assert!(
        h.credentials
            .load_account("alice@acme.com")
            .unwrap()
            .is_none(),
        "the saved sign-in is cleared: a restart asks again"
    );

    // No heartbeats there, ever.
    h.tick(at("10:06:00"));
    h.tick(at("10:08:00"));
    assert!(h.heartbeats().is_empty());

    // Another workspace can still be chosen with the same sign-in.
    h.command(at("10:09:00"), Command::SwitchWorkspace);
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![
        workspace("w1", "Acme"),
        workspace("w2", "Globex"),
    ])));
    h.command(
        at("10:10:00"),
        Command::SwitchToWorkspace {
            index: 1,
            at: at("10:10:00"),
        },
    );
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p9", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(
        h.credentials
            .load_account("alice@acme.com")
            .unwrap()
            .is_some(),
        "saved again for the new workspace"
    );
}

#[test]
fn a_revoked_device_found_by_the_uploader_is_confirmed_with_the_device_status() {
    let mut h = tracking();
    h.control_requests();
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::DeviceRevoked {
        owner,
        workspace_id: wid("w1"),
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::DeviceRevoked);
    assert!(h
        .control_requests()
        .iter()
        .any(|r| matches!(r, ControlRequest::DeviceStatus)));
    assert!(!h.machine.is_tracking_active());
}

#[test]
fn a_locked_keyring_keeps_tracking_offline_and_blocks_wait_for_the_unlock() {
    let first = saved(vec![project("p1", 5)], None);
    let locked = Arc::new(InMemoryCredentialStore::new());
    locked.inject_load_error(true);
    let mut s = setup(
        at("10:00:00"),
        Some(Saved {
            session: first.session.clone(),
            secrets: first.secrets.clone(),
        }),
    );
    s.credentials = locked.clone();
    // The harness cannot pre-store the item through the failing store.
    locked.inject_load_error(false);
    locked.save_account(&first.secrets).unwrap();
    locked.inject_load_error(true);
    let mut h = build(s);
    let snapshot = h.state.snapshot();
    assert_eq!(
        snapshot.status,
        TrackerStatus::Tracking,
        "offline on the cached projects"
    );
    assert!(snapshot.credentials_locked, "the alert offers an unlock");
    assert!(
        !h.control_requests().iter().any(|r| matches!(
            r,
            ControlRequest::UseAccount(Some(_)) | ControlRequest::Heartbeat(_)
        )),
        "heartbeats wait for the unlock"
    );
    assert!(
        !h.upload_commands()
            .iter()
            .any(|c| matches!(c, UploadCommand::Account(Some(_)))),
        "uploads wait too"
    );

    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    assert!(
        h.queued().is_empty(),
        "no signing key yet: the block waits in memory"
    );
    assert_eq!(h.state.snapshot().recent_blocks.len(), 1, "and is shown");

    // The user unlocked the keyring (its own prompt): the block is queued.
    h.credentials.inject_load_error(false);
    h.command(at("10:11:00"), Command::ReloadCredentials);
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:10:00"), Some(EndReason::Interval))]
    );
    assert!(!h.state.snapshot().credentials_locked);
    assert!(h
        .control_requests()
        .iter()
        .any(|r| matches!(r, ControlRequest::UseAccount(Some(_)))));
    let commands = h.upload_commands();
    assert!(commands
        .iter()
        .any(|c| matches!(c, UploadCommand::Account(Some(_)))));
    assert!(commands
        .iter()
        .any(|c| matches!(c, UploadCommand::RowsQueued)));
}

#[test]
fn a_saved_sign_in_without_its_keyring_item_has_expired() {
    let first = saved(vec![project("p1", 5)], None);
    let empty = Arc::new(InMemoryCredentialStore::new());
    let mut s = setup(at("10:00:00"), None);
    s.credentials = empty;
    s.saved = None;
    let mut h = build(s);
    h.worker.start(Some(first.session), None);
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert!(h.state.snapshot().login_error.is_some());
    assert!(h.heartbeats().is_empty());
}

#[test]
fn time_only_tracking_keeps_the_lifecycle_stopped_and_still_queues_blocks() {
    let lifecycle = Arc::new(Mutex::new(TrackerLifecycle::new(ConsentManager::new(
        LocalConsent::Granted(ConsentedPermissions::all_disabled()),
        None,
    ))));
    let tracker = clocked_tracker(at("10:00:00"), SessionKind::X11, lifecycle.clone());
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.consent = LocalConsent::Granted(ConsentedPermissions::all_disabled());
    s.machine = Some(tracker.clone());
    s.lifecycle = Some(lifecycle.clone());
    let mut h = build(s);
    assert_eq!(
        h.status(),
        TrackerStatus::Tracking,
        "time-only consent is consent"
    );
    assert_eq!(
        lifecycle.lock().unwrap().state(),
        TrackerState::Stopped,
        "every acquisition is refused"
    );
    assert!(!tracker.driver.is_input_running() && !tracker.driver.is_usage_running());

    tracker.set(at("10:10:00"));
    h.worker.on_tick();
    h.worker.publish();
    let queued = h.queued();
    assert_eq!(queued.len(), 1, "blocks are still recorded");
    let json = serde_json::to_value(&queued[0]).unwrap();
    assert!(json["keyboardHits"].is_null() && json["mouseClicks"].is_null());
    assert!(json.get("productivity").is_none() && json.get("timeline").is_none());
    assert_eq!(json["runningPrograms"], serde_json::json!([]));
    assert_eq!(
        tracker.acquisition.process_calls() + tracker.acquisition.window_calls(),
        0
    );
}

#[test]
fn a_wayland_session_queues_blocks_without_any_unmeasured_field() {
    let lifecycle = Arc::new(Mutex::new(TrackerLifecycle::new(ConsentManager::new(
        LocalConsent::Granted(defaults()),
        None,
    ))));
    let tracker = clocked_tracker(at("10:00:00"), SessionKind::Wayland, lifecycle.clone());
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.machine = Some(tracker.clone());
    s.lifecycle = Some(lifecycle.clone());
    let mut h = build(s);
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let capabilities = h.state.snapshot().capabilities;
    assert_eq!(
        capabilities.keyboard.state,
        crate::agent::state::CapabilityState::Unsupported
    );
    assert_eq!(
        capabilities.active_window.state,
        crate::agent::state::CapabilityState::Unsupported
    );
    assert_eq!(
        capabilities.running_programs.state,
        crate::agent::state::CapabilityState::On
    );
    assert_eq!(
        capabilities.webcam.state,
        crate::agent::state::CapabilityState::OffByYou
    );

    tracker.set(at("10:10:00"));
    h.worker.on_tick();
    h.worker.publish();
    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    let json = serde_json::to_value(&rows[0].1).unwrap();
    for absent in [
        "productivity",
        "idleSeconds",
        "timeline",
        "injectedInputEvents",
        "webcamJpgBase64",
    ] {
        assert!(json.get(absent).is_none(), "{absent} sent: {json}");
    }
    assert!(
        json["keyboardHits"].is_null() && json["mouseClicks"].is_null(),
        "null, never 0: {json}"
    );
    assert_eq!(json["activeWindows"], serde_json::json!([]));
    assert_eq!(
        json["runningPrograms"].as_array().unwrap().len(),
        2,
        "app-scope programs are kept"
    );
    assert_eq!(json["platform"], "linux");
    assert!(h.heartbeats().iter().all(|s| *s != PresenceStatus::Idle));
}

#[test]
fn an_in_flight_capture_is_discarded_when_consent_is_revoked_before_it_returns() {
    let mut h = tracking();
    h.machine.hold_captures.store(true, Ordering::SeqCst);
    h.machine.input_at(at("10:04:59"));
    h.tick(at("10:05:01"));
    assert_eq!(h.machine.captures(), 1, "asked of dosi-capture");
    // Screenshots are withdrawn while the capture runs.
    let mut no_screens = defaults();
    no_screens.screenshot = false;
    h.command(
        at("10:05:30"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(no_screens),
        },
    );
    h.machine.release_captures();
    h.deliver_captures();
    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    let queued = h.queued();
    assert_eq!(queued.len(), 1);
    assert!(
        queued[0].screenshot_png_base64.is_none(),
        "the late capture is discarded"
    );
    assert!(h.state.snapshot().last_screenshot.is_none());
}

#[test]
fn a_capture_that_returns_after_its_block_ended_is_not_attached_to_the_next() {
    let mut h = tracking();
    h.machine.hold_captures.store(true, Ordering::SeqCst);
    h.machine.input_at(at("10:04:59"));
    h.tick(at("10:05:01"));
    // The block ends (task change) before the capture comes back.
    h.command(
        at("10:06:00"),
        Command::SetTaskNote {
            note: "Other".into(),
            at: at("10:06:00"),
        },
    );
    h.machine.release_captures();
    h.deliver_captures();
    h.machine.input_at(at("10:15:59"));
    h.tick(at("10:16:00"));
    assert!(
        h.queued()
            .iter()
            .all(|a| a.screenshot_captured_at != Some(at("10:05:01"))),
        "never attached to a block it does not show"
    );
}

#[test]
fn a_rotated_refresh_token_is_saved_in_the_keyring() {
    let h = tracking();
    let tokens = h.worker.account.as_ref().unwrap().tokens.clone();
    // A refresh answered with a new refresh token (as the API client does).
    let (now_instant, now_utc) = tokens.current_time();
    tokens.set_granted(crate::api::GrantedToken::new(
        "access".into(),
        Some("refresh-2".into()),
        Some(3600),
        now_instant,
        now_utc,
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let saved = h
            .credentials
            .load_account("alice@acme.com")
            .unwrap()
            .unwrap();
        if saved.refresh_token.as_deref() == Some("refresh-2") {
            assert_eq!(
                saved.device_signing_key,
                Some(KEY.to_vec()),
                "the rest of the item is kept"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the rotated token was not saved"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_autostart_without_resume_at_login_or_an_indicator_waits_for_the_user() {
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.launch.autostart = true;
    s.cfg.tracking.resume_at_login = false;
    let h = build(s);
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert!(!h.machine.is_tracking_active());

    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.launch = Launch {
        autostart: true,
        indicator: false,
    };
    let mut h = build(s);
    assert_eq!(
        h.status(),
        TrackerStatus::Paused,
        "no visible indicator: never tracks by itself"
    );
    h.command(at("10:01:00"), Command::Resume { at: at("10:01:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
}

#[test]
fn a_project_asking_for_more_than_the_consent_is_told_once() {
    let keyboard_only = ConsentedPermissions {
        keyboard: true,
        ..ConsentedPermissions::all_disabled()
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], None)),
        LocalConsent::Granted(keyboard_only),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let asked: Vec<NoticeKind> = h
        .notices()
        .into_iter()
        .filter(|n| matches!(n, NoticeKind::PolicyRequestsConsent { .. }))
        .collect();
    assert_eq!(asked.len(), 1);
    match &asked[0] {
        NoticeKind::PolicyRequestsConsent {
            project_title,
            capabilities,
        } => {
            assert_eq!(project_title, "Project p1");
            assert!(capabilities.contains(&"screenshots".to_owned()));
            assert!(!capabilities.contains(&"keyboard counts".to_owned()));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        !h.machine.is_screenshot_allowed(),
        "nothing more is captured"
    );
    h.worker
        .on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(
        h.notices()
            .iter()
            .filter(|n| matches!(n, NoticeKind::PolicyRequestsConsent { .. }))
            .count(),
        1,
        "told once"
    );
}

#[test]
fn the_webcam_cannot_be_granted_on_linux() {
    let mut h = tracking();
    let with_webcam = ConsentedPermissions {
        webcam: true,
        ..defaults()
    };
    h.command(
        at("10:01:00"),
        Command::UpdateConsent {
            consent: LocalConsent::Granted(with_webcam),
        },
    );
    assert_eq!(
        h.state.snapshot().consent_error.as_deref(),
        Some(WEBCAM_UNAVAILABLE)
    );
    assert_eq!(h.consent_store.save_count(), 0, "nothing is saved");
    assert_eq!(
        h.status(),
        TrackerStatus::Tracking,
        "the consent in force stays"
    );
    assert_eq!(
        h.state.snapshot().capabilities.webcam.state,
        crate::agent::state::CapabilityState::OffByYou
    );
}

#[test]
fn the_session_going_away_ends_the_block_and_quits() {
    let mut h = tracking();
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:06:30"));
    let quit = h.worker.on_system(SystemNotice {
        event: SystemEvent::SessionRemoved,
        at: at("10:06:30"),
        done: None,
    });
    assert!(quit, "the worker quits (design 4, step 5)");
    assert_eq!(
        h.spans(),
        [(at("10:00:00"), at("10:06:30"), Some(EndReason::Shutdown))]
    );
}

#[test]
fn capture_retry_asks_the_portal_again() {
    let mut h = tracking();
    h.command(at("10:01:00"), Command::RetryCapture);
    assert_eq!(h.machine.capture_retries.load(Ordering::SeqCst), 1);
}

#[test]
fn an_archived_workspace_cannot_be_chosen() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    let mut archived = workspace("w2", "Old");
    archived.is_archived = true;
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme"), archived.clone()]),
    });
    assert_eq!(
        used_account(&mut h).workspace_id,
        Some(wid("w1")),
        "the only selectable one is used"
    );

    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("Old"));
    h.worker.on_control(ControlEvent::SignedIn {
        account,
        workspaces: Ok(vec![workspace("w1", "Acme"), archived]),
    });
    assert!(h.state.snapshot().login_error.unwrap().contains("archived"));
    assert!(h.worker.saved.is_none());
}

#[test]
fn parked_and_stripped_uploads_are_told_and_the_projects_checked_again() {
    let mut h = tracking();
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::Parked {
        owner: owner.clone(),
        count: 2,
    });
    h.worker
        .on_upload(UploadEvent::Stripped { owner, count: 1 });
    h.worker.on_upload(UploadEvent::Issue(Some(
        crate::agent::state::UploadIssue::ClockAhead {
            next_attempt_at: at("10:05:00"),
        },
    )));
    h.worker.on_upload(UploadEvent::Synced(at("10:00:30")));
    h.worker.publish();
    let notices = h.notices();
    assert!(notices.contains(&NoticeKind::BlocksNotAccepted { count: 2 }));
    assert!(notices.contains(&NoticeKind::ScreenshotsRemoved { count: 1 }));
    let snapshot: SharedState = h.state.snapshot();
    assert!(matches!(
        snapshot.upload_issue,
        Some(crate::agent::state::UploadIssue::ClockAhead { .. })
    ));
    assert_eq!(snapshot.last_sync, Some(at("10:00:30")));
    h.control_requests();
    h.tick(at("10:00:40"));
    assert!(h
        .control_requests()
        .iter()
        .any(|r| matches!(r, ControlRequest::RefreshProjects)));
}

#[test]
fn the_snapshot_names_why_each_capability_is_off() {
    let keyboard_only = ConsentedPermissions {
        keyboard: true,
        ..ConsentedPermissions::all_disabled()
    };
    let mut s = setup(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)));
    s.consent = LocalConsent::Granted(keyboard_only);
    s.cfg.capture.mouse = false;
    let mut h = build(s);
    let capabilities = h.state.snapshot().capabilities;
    use crate::agent::state::CapabilityState as C;
    assert_eq!(capabilities.keyboard.state, C::On);
    assert_eq!(capabilities.screenshot.state, C::OffByYou);
    assert_eq!(capabilities.mouse.state, C::OffByYou, "consent first");
    assert_eq!(capabilities.webcam.state, C::OffByYou);
    h.command(
        at("10:01:00"),
        Command::Pause {
            resume_after: None,
            at: at("10:01:00"),
        },
    );
    assert_eq!(h.state.snapshot().capabilities.keyboard.state, C::Paused);
}

// ---- The real tracker (fake parts) on a test clock --------------------------------------

/// A [`crate::tracking::tracker::Tracker`] with fake parts on a clock the
/// test moves.
struct ClockedTracker {
    inner: crate::tracking::tracker::Tracker,
    now: Mutex<DateTime<Utc>>,
    driver: Arc<crate::tracking::tracker::FakeSensorDriver>,
    acquisition: Arc<crate::tracking::fake::FakeAcquisitionProvider>,
}

impl ClockedTracker {
    fn set(&self, to: DateTime<Utc>) {
        *self.now.lock().unwrap() = to;
    }
}

/// An idle source saying the user is present now.
struct Present;

impl crate::tracking::traits::IdleSource for Present {
    fn last_input(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, crate::tracking::traits::TrackingError> {
        Ok(Some(now))
    }

    fn kind(&self) -> &'static str {
        "screensaver"
    }
}

fn clocked_tracker(
    start: DateTime<Utc>,
    kind: SessionKind,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
) -> Arc<ClockedTracker> {
    use crate::tracking::tracker::{FakeSensorDriver, Tracker, TrackerParts};
    let driver = Arc::new(FakeSensorDriver::new());
    let acquisition = Arc::new(crate::tracking::fake::FakeAcquisitionProvider::new());
    let inner = Tracker::new(TrackerParts {
        kind,
        desktop: Some("GNOME".into()),
        lifecycle,
        device: FakeMachine::new(start).device.clone(),
        driver: driver.clone(),
        active_window: acquisition.clone(),
        running_apps: acquisition.clone(),
        screen: None,
        idle: Some(Arc::new(Present)),
    });
    Arc::new(ClockedTracker {
        inner,
        now: Mutex::new(start),
        driver,
        acquisition,
    })
}

impl Machine for ClockedTracker {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }

    fn awake_millis(&self) -> u64 {
        (*self.now.lock().unwrap() - at("00:00:00")).num_milliseconds() as u64
    }

    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.inner.last_input(now)
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.inner.first_input_after(since, now)
    }

    fn set_capture_sink(&self, sink: UnboundedSender<CaptureEvent>) {
        self.inner.set_capture_sink(sink)
    }

    fn request_capture(&self, block: BlockId, blur: bool) -> bool {
        self.inner.request_capture(block, blur)
    }

    fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        self.inner.capture_screen(blur)
    }

    fn snapshot(
        &self,
        block: &BlockSpec<'_>,
        screen: Option<ProvenanceScreenCapture>,
    ) -> BuiltBlock {
        self.inner.snapshot(block, screen)
    }

    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        self.inner
            .kept_idle_blocks(project_id, from, to, interval, description)
    }

    fn device(&self) -> &DeviceInfo {
        self.inner.device()
    }

    fn current_app(&self) -> Option<String> {
        self.inner.current_app()
    }

    fn random_fraction(&self) -> f64 {
        0.5
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&ConsentedPermissions>) {
        self.inner.apply_sensor_policy(active, perms)
    }

    fn session_summary(&self) -> SessionSummary {
        self.inner.session_summary()
    }

    fn platform(&self) -> PlatformCapabilities {
        self.inner.platform()
    }

    fn retry_capture_permission(&self) {
        self.inner.retry_capture_permission()
    }
}
