use std::sync::Mutex;

use chrono::FixedOffset;

use super::control::Farewell;
use super::*;
use crate::config::{ConsentedPermissions, LocalConsent, MemoryConsentStore};
use crate::tracking::{build_activity, Measurements, Zone};

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// 5 October 2026 at `hms`, UTC.
fn at(hms: &str) -> DateTime<Utc> {
    utc(&format!("2026-10-05T{hms}Z"))
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
    assert_eq!(days[0], (0, utc("2026-09-20T18:00:00Z"), utc("2026-09-21T18:00:00Z")));
    assert_eq!(days[2], (2, utc("2026-09-22T18:00:00Z"), utc("2026-09-23T18:00:00Z")));

    // On a Monday nothing is past yet.
    assert!(past_days_of_week(utc("2026-09-21T09:00:00Z"), &Utc).is_empty());
    // On a Sunday, six days are.
    assert_eq!(past_days_of_week(utc("2026-09-27T09:00:00Z"), &Utc).len(), 6);
}

#[test]
fn only_the_part_of_a_block_after_the_cutoff_is_counted() {
    let midnight = utc("2026-09-30T18:00:00Z");
    // Entirely after: all of it.
    assert_eq!(tracked_secs_since(utc("2026-09-30T18:10:00Z"), utc("2026-09-30T18:15:00Z"), midnight), 300.0);
    // Spanning midnight: only the part after it.
    assert_eq!(tracked_secs_since(utc("2026-09-30T17:58:00Z"), utc("2026-09-30T18:03:30Z"), midnight), 210.0);
    // Entirely before: nothing.
    assert_eq!(tracked_secs_since(utc("2026-09-30T17:00:00Z"), utc("2026-09-30T17:05:00Z"), midnight), 0.0);
}

// ---- Signing in ------------------------------------------------------------------------

#[test]
fn only_a_refusal_counts_as_a_rejected_sign_in() {
    // Anything but the server refusing the credentials — here a plain
    // failure, as when the server cannot be reached — keeps the sign-in.
    assert!(!is_auth_rejection(&AuthError::Offline("tcp connect error".into())));
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
    let refused = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
    let mut client = ApiClient::new(one_shot_server("400 Bad Request", refused), "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("the server refuses");
    assert!(is_auth_rejection(&error));
    assert_eq!(friendly_login_error(&error), "Invalid username or password!");
}

#[tokio::test]
async fn a_server_error_is_not_a_rejected_sign_in() {
    let mut client = ApiClient::new(one_shot_server("503 Service Unavailable", ""), "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("the server fails");
    assert!(!is_auth_rejection(&error));
}

#[tokio::test]
async fn an_unreachable_server_is_not_a_rejected_sign_in() {
    // Nothing listens on port 9 (discard) locally: a real connect error.
    let mut client = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("nothing is listening");
    assert!(!is_auth_rejection(&error));
}

#[test]
fn accounts_never_show_their_password() {
    let account = Account::new("https://t.example".into(), "a@acme.com".into(), "s3cr3t".into()).in_workspace("w1", "Acme");
    let shown = format!("{account:?}");
    assert!(!shown.contains("s3cr3t") && shown.contains("a@acme.com"), "{shown}");
}

// ---- The window's lists ------------------------------------------------------------------

fn block(row_id: i64) -> RecentBlock {
    let at = utc("2026-09-30T10:00:00Z");
    RecentBlock {
        row_id,
        project: "Website".into(),
        started_at: at,
        ended_at: at,
        productivity: 50,
        keyboard_hits: 0,
        mouse_clicks: 0,
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
    assert_eq!(clean_note("  Invoice   export\n\tfor ACME \u{7} "), Some("Invoice export for ACME".into()));
    assert_eq!(clean_note(" \n "), None);
    let long = "word ".repeat(100);
    let kept = clean_note(&long).unwrap();
    assert!(kept.chars().count() <= TASK_NOTE_MAX_CHARS);
    assert!(!kept.ends_with(' '));
}

// ---- The worker, driven through a fake machine -----------------------------------------

/// A machine whose clocks, input and screen the test controls.
struct FakeMachine {
    now: Mutex<DateTime<Utc>>,
    awake_ms: Mutex<u64>,
    inputs: Mutex<Vec<DateTime<Utc>>>,
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

    fn captures(&self) -> u32 {
        *self.captures.lock().unwrap()
    }

    fn is_tracking_active(&self) -> bool {
        self.input_running.load(std::sync::atomic::Ordering::SeqCst)
            || self.usage_running.load(std::sync::atomic::Ordering::SeqCst)
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
        self.inputs.lock().unwrap().iter().copied().filter(|t| *t <= now).max()
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.inputs.lock().unwrap().iter().copied().filter(|t| *t > since && *t <= now).min()
    }

    fn capture_screen(&self, blur: bool) -> Option<ScreenCapture> {
        *self.captures.lock().unwrap() += 1;
        Some(ScreenCapture {
            jpeg_base64: "/9j/screen".into(),
            thumb_jpg_base64: Some("/9j/dGh1bWI=".into()),
            captured_at: self.now(),
            blurred: blur,
        })
    }

    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<ScreenCapture>) -> Activity {
        self.snapshot_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(hook) = self.on_snapshot.lock().unwrap().as_ref() {
            hook();
        }
        let window_now = if block.perms.active_window && self.usage_running.load(std::sync::atomic::Ordering::SeqCst) {
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
        let running_programs = if block.perms.running_programs && self.usage_running.load(std::sync::atomic::Ordering::SeqCst) {
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
        let webcam_jpg_base64 = if block.perms.webcam && self.is_tracking_active() && block.captures_at_end() {
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
        Some("code.exe".into())
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
            }
        } else if self.input_running.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.input_stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    let mut saved = Credentials::new("alice@acme.com".into(), "pw".into(), None);
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
    let mut cfg = AppConfig::default();
    cfg.consent = consent;
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
    let mut cfg = AppConfig::default();
    cfg.consent = consent;
    cfg.capture.webcam = true;
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
        storage::owner_key("alice@acme.com")
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
}

#[test]
fn a_saved_sign_in_starts_tracking_at_once_on_the_saved_projects() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p2"))), false, None);
    assert_eq!(h.status(), TrackerStatus::Tracking, "offline first: no waiting for the server");
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.selected_project.as_deref(), Some("p2"), "the remembered project");
    assert_eq!(snapshot.tracking_since, Some(at("10:00:00")));
    assert_eq!(snapshot.projects.len(), 2);
    assert_eq!(snapshot.display_name, "alice@acme.com");

    let requests = h.control_requests();
    assert!(matches!(&requests[0], ControlRequest::UseAccount(Some(account)) if account.username == "alice@acme.com"));
    assert!(matches!(requests[1], ControlRequest::RefreshProjects), "re-validated with the server");
    assert!(requests.iter().any(|r| matches!(r, ControlRequest::Heartbeat(h) if h.status == PresenceStatus::Tracking)));
    assert!(matches!(&h.upload_commands()[0], UploadCommand::Account(Some(_))));
}

#[test]
fn several_projects_and_none_remembered_ask_the_user_to_choose() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("acme", 5), project("zeta", 5)], None)), false, None);
    assert_eq!(h.status(), TrackerStatus::ChooseProject, "never silently the first one");
    assert!(h.tracking_since().is_none());
    assert!(h.state.snapshot().notices.iter().any(|n| n.kind == NoticeKind::ChooseProject));
    assert!(h.heartbeats().contains(&PresenceStatus::Choosing));

    h.command(at("10:01:00"), Command::SelectProject { project_id: "zeta".into(), at: at("10:01:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:01:00")));
    assert_eq!(h.worker.saved.as_ref().unwrap().project_id.as_deref(), Some("zeta"), "remembered");
}

#[test]
fn without_a_recent_project_list_it_waits_for_the_server() {
    let mut old = saved(vec![project("p1", 5)], Some("p1"));
    old.projects.as_mut().unwrap().resolved_at = at("10:00:00") - TimeDelta::days(8);
    let mut h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(h.status(), TrackerStatus::Connecting, "a list older than a week is not used");

    h.worker.on_control(ControlEvent::Reachable(false));
    h.worker.on_control(ControlEvent::ProjectsFailed(ApiError::Offline("tcp connect error".into())));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Offline);

    h.machine.set(at("10:03:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:03:00")));
    assert!(h.state.snapshot().server_reachable);
    // The fresh list is saved for the next offline start.
    assert_eq!(h.worker.saved.as_ref().unwrap().projects.as_ref().unwrap().resolved_at, at("10:03:00"));
}

#[test]
fn the_block_ends_where_the_user_paused_and_the_pause_survives_a_restart() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    credentials::save_at(&file, &saved(vec![project("p1", 5)], None)).unwrap();
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));

    // Clicked at 10:05; the worker got to it two seconds later.
    h.command(at("10:05:02"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Pause))]);
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(h.state.snapshot().paused_since, Some(at("10:05:00")));
    assert!(matches!(h.upload_commands().last(), Some(UploadCommand::RowsQueued)));

    // Restarted by Windows at sign-in: still paused.
    let mut restarted = harness_at(at("18:00:00"), credentials::load_from(&file), true, Some(file.clone()));
    assert_eq!(restarted.status(), TrackerStatus::Paused);
    assert!(restarted.tracking_since().is_none());
    assert!(restarted.heartbeats().contains(&PresenceStatus::Paused));
    restarted.command(at("18:01:00"), Command::Resume { at: at("18:01:00") });
    assert_eq!(restarted.tracking_since(), Some(at("18:01:00")), "a fresh block from the click");
    assert!(credentials::load_from(&file).unwrap().pause.is_none(), "resuming is saved too");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_timed_break_never_resumes_silently_after_an_autostart() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    let mut on_break = saved(vec![project("p1", 5)], None);
    on_break.pause = Some(SavedPause { since: at("09:50:00"), until: Some(at("10:05:00")) });
    credentials::save_at(&file, &on_break).unwrap();

    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), true, Some(file.clone()));
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(h.state.snapshot().resume_at, None, "paused until the user resumes");
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
    assert_eq!(h.tracking_since(), Some(at("10:05:00")), "from the end of the break");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_break_that_ends_while_the_user_is_away_resumes_with_their_return() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Break))]);
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:20:00")));

    // Nobody touched the computer: the break ends, tracking waits.
    h.tick(at("10:20:01"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(h.tracking_since().is_none());

    h.machine.input_at(at("10:31:10"));
    h.tick(at("10:31:11"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:31:10")));
    assert!(h.state.snapshot().away_prompt.is_none(), "a break is not time to ask about");
}

#[test]
fn a_break_that_runs_out_during_a_sleep_resumes_at_the_wake_not_inside_the_sleep() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    h.system(SystemEvent::Suspending, at("10:10:00"));
    h.machine.sleep_until(at("13:00:00"));
    h.worker.on_system(SystemNotice { event: SystemEvent::Resumed, at: at("13:00:00"), done: None });
    // The key press that woke it is recent: someone is there.
    h.machine.input_at(at("13:00:01"));
    h.tick(at("13:00:02"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("13:00:00")), "from the wake, not from the end of the break");
    h.machine.input_at(at("13:10:30"));
    h.tick(at("13:10:31"));
    assert!(h.queued().iter().skip(1).all(|block| block.started_at >= at("13:00:00")), "{:?}", h.spans());
}

#[test]
fn a_break_that_runs_out_during_a_sleep_nobody_announced_resumes_after_it() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
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
    h.worker.on_upload(UploadEvent::SignInExpired { owner, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert!(h.state.snapshot().away_prompt.is_some(), "the question stays");
    h.command(at("10:40:05"), Command::ResolveAway { keep: true });
    let kept: Vec<Activity> = h.queued().into_iter().filter(|a| a.end_reason == Some(EndReason::Idle) && a.started_at >= at("10:02:00")).collect();
    assert_eq!(kept.first().map(|a| a.started_at), Some(at("10:02:00")));
    assert_eq!(kept.last().map(|a| a.ended_at), Some(at("10:40:00")));
}

#[test]
fn upload_events_of_the_previous_account_are_ignored() {
    let mut h = tracking();
    let other = storage::owner_key("bob@acme.com");
    h.worker.on_upload(UploadEvent::Projects { owner: other.clone(), projects: vec![project("p9", 5)] });
    h.worker.on_upload(UploadEvent::SignInExpired { owner: other, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.state.snapshot().projects.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p1"]);
}

#[test]
fn going_idle_ends_the_block_at_the_last_input_and_asks_about_the_time_away() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking, "not idle for five minutes yet");
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert_eq!(h.state.snapshot().away_since, Some(at("10:02:00")));
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:02:00"), Some(EndReason::Idle))]);
    assert!(queued[0].screenshot_png_base64.is_none(), "no capture for an idle end");
    assert!(h.heartbeats().contains(&PresenceStatus::Idle));

    // Back at 10:30: a new block starts there, and the question is asked.
    h.machine.input_at(at("10:30:00"));
    h.tick(at("10:30:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:30:00")));
    let prompt = h.state.snapshot().away_prompt.expect("asked");
    assert_eq!((prompt.from, prompt.to, prompt.minutes()), (at("10:02:00"), at("10:30:00"), 28));
    assert_eq!(prompt.project_title, "Project p1");

    // Kept: 28 minutes as idle blocks of at most one interval, with the note.
    h.command(at("10:30:05"), Command::SetTaskNote { note: "Client call".into(), at: at("10:30:05") });
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
        assert!(block.timeline.is_empty() && block.active_windows.is_empty() && !block.has_captures());
        assert_eq!(block.description.as_deref(), Some("Client call"));
        assert_eq!(block.project_id, "p1");
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
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 15)], None)), false, None);
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:10:01"));
    assert!(h.queued().is_empty(), "the 10:00-10:10 block waits for the user");
    assert_eq!(h.state.snapshot().unsaved_since, Some(at("10:00:00")), "and still counts in the live totals");
    h.tick(at("10:22:00"));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:00"), Some(EndReason::Idle))]);
}

#[test]
fn locking_ends_the_block_without_a_capture_and_unlocking_starts_a_new_one() {
    let mut h = tracking();
    // Locked before the block's screenshot moment (50%: 10:05).
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));
    h.system(SystemEvent::Locked, at("10:04:30"));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:30"), Some(EndReason::Lock))]);
    assert!(h.queued()[0].screenshot_png_base64.is_none(), "never a capture of a locked screen");
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
    // Reconnected, but still on the lock screen.
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
    h.worker.on_system(SystemNotice { event: SystemEvent::Suspending, at: at("10:06:00"), done: Some(done) });
    assert!(saved.try_recv().is_ok(), "Windows is told the block is saved");
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]);
    assert!(h.upload_commands().is_empty(), "no network before sleep");

    // Three hours later.
    h.machine.sleep_until(at("13:06:00"));
    h.worker.on_system(SystemNotice { event: SystemEvent::Resumed, at: at("13:06:00"), done: None });
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.tracking_since(), Some(at("13:06:00")), "a new block from the wake");
    assert_eq!(h.queued().len(), 1, "the sleep is not reported twice");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::RowsQueued)));
}

#[test]
fn a_sleep_nobody_announced_ends_the_block_at_the_tick_before_it() {
    let mut h = tracking();
    h.machine.input_at(at("10:03:59"));
    h.tick(at("10:04:00"));
    // The lid closed; Windows said nothing. Three hours on the wall clock,
    // two seconds of awake time.
    h.machine.sleep_until(at("13:04:00"));
    h.machine.set(at("13:04:02"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:00"), Some(EndReason::Sleep))]);
    assert_eq!(h.tracking_since(), Some(at("13:04:02")));
}

#[test]
fn a_sleep_that_never_came_does_not_stop_tracking_for_good() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.control_requests();
    h.system(SystemEvent::Suspending, at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(h.heartbeats().is_empty(), "nothing goes over the network before sleep");
    // No resume was heard, but the user types: the machine is awake.
    h.machine.input_at(at("10:06:40"));
    h.tick(at("10:06:41"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:06:40")));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]);
}

#[test]
fn the_session_ending_saves_the_block_and_a_cancelled_shutdown_goes_on() {
    let mut h = tracking();
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:07:00"));
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    h.machine.set(at("10:07:30"));
    h.worker.on_system(SystemNotice { event: SystemEvent::SessionEnding, at: at("10:07:30"), done: Some(done) });
    assert!(saved.try_recv().is_ok());
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:30"), Some(EndReason::Shutdown))]);
    // A second notice (WM_ENDSESSION after WM_QUERYENDSESSION) changes nothing.
    h.system(SystemEvent::SessionEnding, at("10:07:31"));
    assert_eq!(h.queued().len(), 1);

    h.system(SystemEvent::SessionContinues, at("10:08:00"));
    assert_eq!(h.tracking_since(), Some(at("10:08:00")));
}

#[test]
fn switching_project_ends_the_block_at_the_click() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))), false, None);
    h.machine.input_at(at("10:05:00"));
    h.command(at("10:06:01"), Command::SelectProject { project_id: "p2".into(), at: at("10:06:00") });
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]);
    assert_eq!(queued[0].project_id, "p1", "the work so far belongs to the project it was done on");
    assert_eq!(h.state.snapshot().selected_project.as_deref(), Some("p2"));
    assert_eq!(h.tracking_since(), Some(at("10:06:00")));
}

#[test]
fn a_project_that_disappears_stops_tracking_or_moves_to_the_only_one_left() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))), false, None);
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:06:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p2", 5)]));
    h.worker.publish();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]);
    assert_eq!(h.state.snapshot().selected_project.as_deref(), Some("p2"), "exactly one left");
    assert!(h.state.snapshot().notices.iter().any(|n| matches!(&n.kind,
        NoticeKind::ProjectUnavailable { title, now_tracking: Some(next) } if title == "Project p1" && next == "Project p2")));

    // Gone again, with two others left: choose.
    h.machine.set(at("10:20:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p3", 5), project("p4", 5)]));
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
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:10:00"), Some(EndReason::Interval))]);
    assert_eq!(h.state.snapshot().next_capture_at, Some(at("10:15:00")));
}

#[test]
fn a_new_task_note_ends_the_block_or_relabels_a_young_one() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:30"));
    h.command(at("10:00:30"), Command::SetTaskNote { note: "Invoices".into(), at: at("10:00:30") });
    assert!(h.queued().is_empty(), "a block under a minute takes the note");
    assert_eq!(h.state.snapshot().task_note, "Invoices");

    h.machine.input_at(at("10:05:00"));
    h.command(at("10:05:00"), Command::SetTaskNote { note: "Client call".into(), at: at("10:05:00") });
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Task))]);
    assert_eq!(queued[0].description.as_deref(), Some("Invoices"));
    assert_eq!(h.tracking_since(), Some(at("10:00:00")), "tracking goes on");
    assert_eq!(h.state.snapshot().block_started_at, Some(at("10:05:00")));

    // The same note again is no change.
    h.command(at("10:06:00"), Command::SetTaskNote { note: " Client  call ".into(), at: at("10:06:00") });
    assert_eq!(h.queued().len(), 1);
}

#[test]
fn sync_now_saves_a_block_over_a_minute_old_and_retries_uploads() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:20"));
    h.command(at("10:00:30"), Command::SyncNow { at: at("10:00:30") });
    assert!(h.queued().is_empty(), "under a minute: keeps running");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Now)));

    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:00"), Command::SyncNow { at: at("10:04:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:00"), Some(EndReason::Interval))]);
    assert_eq!(h.state.snapshot().next_capture_at, Some(at("10:14:00")), "a full interval from the cut");
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
    assert_eq!(h.machine.captures(), 1, "one per block");
    let preview = h.state.snapshot().last_screenshot.expect("shown in the window");
    assert_eq!(preview.taken_at, at("10:05:01"), "when it was taken, not when the block was saved");

    // Paused at 10:12, before the next block's moment (10:15): taken then.
    h.machine.input_at(at("10:12:00"));
    h.command(at("10:12:00"), Command::Pause { resume_after: None, at: at("10:12:00") });
    assert_eq!(h.machine.captures(), 2);
    assert_eq!(h.queued()[1].screenshot_captured_at, Some(at("10:12:00")));
}

#[test]
fn a_refused_password_stops_tracking_keeps_the_queue_and_asks_for_it_again() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.machine.set(at("10:05:00"));
    h.control_requests();
    h.upload_commands();
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::SignInExpired { owner, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::SignOut))]);
    let snapshot = h.state.snapshot();
    assert!(snapshot.login_error.unwrap().contains("Invalid username or password!"));
    assert_eq!((snapshot.display_name.as_str(), snapshot.workspace.as_str()), ("alice@acme.com", "Acme"), "prefilled");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(None))));
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::UseAccount(None))));
    assert!(h.heartbeats().is_empty(), "no heartbeats without a sign-in");

    // The password, entered again.
    h.command(at("10:10:00"), Command::SignIn { workspace: None, username: "alice@acme.com".into(), password: "new".into(), server: None });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, AppConfig::default().api_base_url);
    // The saved workspace is picked again among several.
    let workspaces = vec![Workspace { id: "w0".into(), name: "Other".into(), role: None, plan_name: None, is_owner: false },
        Workspace { id: "w1".into(), name: "Acme".into(), role: None, plan_name: None, is_owner: false }];
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(workspaces) });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:10:00")));
    assert_eq!(h.worker.saved.as_ref().unwrap().password, "new");
    assert_eq!(h.queued().len(), 1, "the earlier block is still queued for this account");
}

#[test]
fn a_sign_in_with_a_server_address_saves_it() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    h.command(at("10:00:00"), Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("ftp://nope".into()),
    });
    assert!(h.state.snapshot().login_error.unwrap().contains("https://"), "an unusable address is refused at once");
    assert!(h.control_requests().is_empty());

    h.command(at("10:00:05"), Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("tracker.acme.com/".into()),
    });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, "https://tracker.acme.com");
    let only = Workspace { id: "w1".into(), name: "Acme".into(), role: None, plan_name: None, is_owner: false };
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![only]) });
    h.worker.on_control(ControlEvent::Projects(vec![project("a", 5), project("b", 5)]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::ChooseProject, "a first sign-in with several projects asks");
    assert_eq!(snapshot.server_url, "https://tracker.acme.com");
    assert_eq!(h.worker.saved.as_ref().unwrap().server.as_deref(), Some("https://tracker.acme.com"));
}

#[test]
fn signing_out_saves_the_block_says_goodbye_and_forgets_the_account() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:31"), Command::SignOut { at: at("10:04:30") });
    let farewell = h.farewells.try_recv().expect("a last heartbeat");
    assert_eq!(farewell.heartbeat.map(|hb| hb.status), Some(PresenceStatus::SignedOut));
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::SignOut { .. })));
    // Saved for the account, which uploads it when it signs in again.
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:30"), Some(EndReason::SignOut))]);
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert!(snapshot.display_name.is_empty() && snapshot.projects.is_empty() && snapshot.recent_blocks.is_empty());
    assert_eq!(snapshot.pending_uploads, 0, "the next account does not see this one's queue");
}

#[tokio::test]
async fn quitting_saves_the_block_at_the_click_and_says_stopped() {
    let mut h = tracking();
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:07:01"));
    let mut farewells = std::mem::replace(&mut h.farewells, tokio::sync::mpsc::unbounded_channel().1);
    let answered = tokio::spawn(async move {
        let farewell = farewells.recv().await.expect("a last heartbeat");
        let status = farewell.heartbeat.as_ref().map(|hb| hb.status);
        let _ = farewell.done.send(());
        status
    });
    h.worker.quit(at("10:07:00")).await;
    assert_eq!(answered.await.unwrap(), Some(PresenceStatus::Stopped));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:00"), Some(EndReason::Quit))]);
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Shutdown { .. })));
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
    assert_eq!(heartbeat.device_id, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11");
    assert_eq!(heartbeat.project_id.as_deref(), Some("p1"));
    assert_eq!(heartbeat.current_app.as_deref(), Some("code.exe"), "the project records the active window");
    assert_eq!(heartbeat.agent_version.as_deref(), Some("9.9.9"));

    // No repeat within the minute while nothing changes...
    h.machine.input_at(at("10:00:30"));
    h.tick(at("10:00:30"));
    assert!(h.heartbeats().is_empty());
    // ...but at once on a change, without the app while paused.
    h.command(at("10:01:30"), Command::Pause { resume_after: None, at: at("10:01:30") });
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
    let h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), true, None);
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.state.snapshot().notices.iter().any(|n| matches!(&n.kind,
        NoticeKind::TrackingStartedAutomatically { project_title } if project_title == "Project p1")));
    let manual = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), false, None);
    assert!(manual.state.snapshot().notices.is_empty());
}

// ---- Against a live backend ------------------------------------------------------------

/// End-to-end against a live backend: real input + window sampling and
/// screenshot → local queue → upload → today/week totals. Run with
/// `DOSI_E2E_URL`, `DOSI_E2E_WORKSPACE`, `DOSI_E2E_USER`,
/// `DOSI_E2E_PASSWORD`, `DOSI_E2E_PROJECT` set:
/// `cargo test e2e_ -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "needs a running backend and a desktop session"]
async fn e2e_capture_upload_and_totals() {
    let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
    let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
    let account = Account::new(cfg.api_base_url.clone(), env("DOSI_E2E_USER"), env("DOSI_E2E_PASSWORD"));
    let mut client = account.client(&ServerClock::default());
    client.login().await.expect("sign-in");
    let wanted = env("DOSI_E2E_WORKSPACE");
    let workspaces = client.workspaces_mine().await.expect("workspaces");
    let workspace = workspaces.iter().find(|w| w.id == wanted || w.name.eq_ignore_ascii_case(&wanted)).expect("the workspace is listed");
    client.set_workspace(Some(workspace.id.clone()));
    let projects = client.my_projects().await.expect("projects");
    let project = projects.iter().find(|p| p.id == env("DOSI_E2E_PROJECT")).expect("the project is listed");
    let session = session_for(project, &cfg);

    let tracker = Tracker::new();
    let path = std::env::temp_dir().join(format!("dosi-e2e-{}.db", uuid::Uuid::new_v4()));
    let store = Storage::open(&path.to_string_lossy(), None).unwrap();
    let owner = account.owner();

    // Two minute buckets' worth of real sampling.
    let started = Utc::now();
    tokio::time::sleep(Duration::from_secs(125)).await;
    let ended = Utc::now();
    let screen = Machine::capture_screen(&tracker, session.policy.perms.blur_screenshots);
    let spec = BlockSpec {
        project_id: session.project_id(),
        perms: &session.policy.perms,
        started_at: started,
        ended_at: ended,
        end_reason: Some(EndReason::Interval),
        description: Some("e2e"),
    };
    let activity = Machine::snapshot(&tracker, &spec, screen);
    store.enqueue(&activity, &owner, &workspace.id).unwrap();
    let queued = store.pending_batch(&owner, 0, 1).unwrap().remove(0).1;
    println!(
        "queued: {} windows, {} timeline minutes, screenshot {}, thumb {}, keys {}, clicks {}, {:?}%",
        queued.active_windows.len(),
        queued.timeline.len(),
        queued.screenshot_png_base64.is_some(),
        queued.screenshot_thumb_jpg_base64.is_some(),
        queued.keyboard_hits,
        queued.mouse_clicks,
        queued.productivity,
    );
    assert_eq!(queued.timeline.len(), 2);
    assert!(queued.active_windows.iter().all(|w| w.seconds.is_some()));

    client.submit_activity(&queued).await.expect("uploaded");
    store.remove_synced(store.pending_batch(&owner, 0, 1).unwrap()[0].0).unwrap();
    let now = Utc::now();
    let (today_start, _) = local_day_and_week_start(now, &Local);
    let today = client.summary(today_start, now, crate::tracking::system::time_zone().as_deref()).await.unwrap();
    println!("server total today: {:.1} min", today.total_tracked_minutes);
    assert!(today.total_tracked_minutes >= 2.0, "today's total includes the block");
    drop(store);
    let _ = std::fs::remove_file(&path);
}

/// End-to-end drive of the worker loop through the commands the window and
/// tray send: sign-in, a timed pause that saves the block and resumes by
/// itself, an open-ended pause + resume, a too-short "Sync now", and quit.
/// Uses the per-user queue and saved sign-in, so run it only on a machine
/// whose tracker data may be replaced. Same env vars as
/// [`e2e_capture_upload_and_totals`].
#[test]
#[ignore = "needs a running backend; replaces the saved sign-in"]
fn e2e_pause_resume_flow() {
    use std::time::Instant;
    let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
    let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
    let state = StateHandle::new();
    let worker = spawn(state.clone(), cfg, Arc::new(Tracker::new()), Launch::default(), Arc::new(|| {}));
    let tx = worker.commands.clone();
    let wait_for = |what: &str, secs: u64, done: &dyn Fn(&crate::state::SharedState) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = state.snapshot();
            if done(&s) {
                return s;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {:?}", s.status);
            std::thread::sleep(Duration::from_millis(200));
        }
    };

    tx.send(Command::SignIn {
        workspace: std::env::var("DOSI_E2E_WORKSPACE").ok(),
        username: env("DOSI_E2E_USER"),
        password: env("DOSI_E2E_PASSWORD"),
        server: None,
    })
    .unwrap();
    let s = wait_for("tracking", 30, &|s| {
        matches!(s.status, TrackerStatus::Tracking | TrackerStatus::ChooseProject)
    });
    if s.status == TrackerStatus::ChooseProject {
        tx.send(Command::SelectProject { project_id: env("DOSI_E2E_PROJECT"), at: Utc::now() }).unwrap();
    }
    let s = wait_for("tracking", 30, &|s| s.status == TrackerStatus::Tracking);
    println!("signed in, tracking since {:?}, block saved at {:?}", s.tracking_since, s.next_capture_at);

    // Work for over a minute, then take a 5-second timed pause.
    std::thread::sleep(Duration::from_secs(65));
    let synced_before = state.snapshot().last_sync;
    tx.send(Command::Pause { resume_after: Some(Duration::from_secs(5)), at: Utc::now() }).unwrap();
    let s = wait_for("paused", 20, &|s| s.status == TrackerStatus::Paused);
    assert!(s.resume_at.is_some() && s.tracking_since.is_none());
    let s = wait_for("the paused block uploaded", 60, &|s| s.last_sync != synced_before && s.pending_uploads == 0);
    println!("paused; block saved and uploaded; today {:.0}s", s.tracked_today_secs);
    let s = wait_for("auto-resume", 20, &|s| s.status == TrackerStatus::Tracking);
    assert!(s.resume_at.is_none() && s.paused_since.is_none());
    println!("auto-resumed");

    // Open-ended pause, then an explicit resume.
    tx.send(Command::Pause { resume_after: None, at: Utc::now() }).unwrap();
    let s = wait_for("paused again", 20, &|s| s.status == TrackerStatus::Paused);
    assert!(s.resume_at.is_none());
    tx.send(Command::Resume { at: Utc::now() }).unwrap();
    wait_for("resumed", 20, &|s| s.status == TrackerStatus::Tracking);
    println!("paused and resumed");

    // A block under a minute is not cut by "Sync now".
    let block = state.snapshot().block_started_at;
    tx.send(Command::SyncNow { at: Utc::now() }).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(state.snapshot().block_started_at, block, "short block kept running");
    println!("short sync kept the block");

    tx.send(Command::Shutdown { at: Utc::now() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.thread.is_finished() {
        assert!(Instant::now() < deadline, "worker did not shut down");
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("shut down cleanly");
}

#[test]
fn starting_a_stretch_resets_the_clocks_and_clears_a_pause() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(30 * 60)), at: at("10:05:00") });
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:35:00")));
    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    let s = h.state.snapshot();
    assert_eq!(s.status, TrackerStatus::Tracking);
    assert!(s.paused_since.is_none() && s.resume_at.is_none());
    assert_eq!((s.tracking_since, s.block_started_at), (Some(at("10:10:00")), Some(at("10:10:00"))));
    assert_eq!(s.next_capture_at, Some(at("10:20:00")), "a full interval until the block is saved");
}

// ---- Choosing the workspace -----------------------------------------------------------

fn workspace(id: &str, name: &str) -> Workspace {
    Workspace { id: id.into(), name: name.into(), role: Some("Member".into()), plan_name: None, is_owner: false }
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

#[test]
fn no_workspace_asks_to_create_one_or_be_invited() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![]) });
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
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme")]) });
    let used = used_account(&mut h);
    assert_eq!((used.workspace_id.as_deref(), used.workspace_name.as_str()), (Some("w1"), "Acme"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!((saved.workspace_id.as_deref(), saved.workspace_name.as_str()), (Some("w1"), "Acme"));
    assert_eq!(h.state.snapshot().workspace, "Acme");
    h.machine.input_at(at("10:04:30"));
    h.tick(at("10:05:00"));
    h.command(at("10:05:00"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert_eq!(h.worker.store.pending_rows(&h.owner(), 0, 10).unwrap()[0].workspace_id, "w1", "blocks are queued in their workspace");
}

#[test]
fn several_workspaces_are_offered_with_their_roles_and_the_chosen_one_is_used() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    let mut globex = workspace("w2", "Globex");
    globex.role = Some("Owner".into());
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), globex]) });
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.workspace_choices.len(), 2);
    assert_eq!(snapshot.workspace_choices[1].role.as_deref(), Some("Owner"));
    assert!(!snapshot.signing_in && snapshot.status == TrackerStatus::SignedOut);
    assert!(h.worker.saved.is_none(), "nothing saved before a workspace is chosen");

    h.command(at("10:00:10"), Command::ChooseWorkspace { index: 1 });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w2"));
    assert!(h.state.snapshot().workspace_choices.is_empty());
    assert_eq!(h.worker.saved.as_ref().unwrap().workspace_id.as_deref(), Some("w2"));
}

#[test]
fn a_named_workspace_is_picked_without_asking() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("globex"));
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]) });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w2"));

    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("w1"));
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]) });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w1"), "by id too");
}

#[test]
fn going_back_from_the_picker_forgets_the_sign_in() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("a", "A"), workspace("b", "B")]) });
    h.command(at("10:00:05"), Command::CancelWorkspaceChoice);
    assert!(h.worker.pending.is_none() && h.worker.choices.is_empty());
    assert!(h.state.snapshot().workspace_choices.is_empty());
    h.command(at("10:00:06"), Command::ChooseWorkspace { index: 0 });
    assert!(h.control_requests().is_empty());
}

#[test]
fn a_saved_sign_in_of_an_older_build_must_sign_in_again() {
    let mut old = saved(vec![project("p1", 5)], None);
    old.workspace_id = None;
    let h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
}

fn list_workspaces(h: &mut Harness) {
    h.control_requests();
    h.upload_commands();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    assert!(h.state.snapshot().finding_workspaces);
    assert!(matches!(h.control_requests().pop(), Some(ControlRequest::MyWorkspaces)));
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")])));
    let snapshot = h.state.snapshot();
    assert!(snapshot.workspace_choices[0].is_current && !snapshot.workspace_choices[1].is_current);
}

#[test]
fn switching_workspace_saves_the_block_and_keeps_the_sign_in() {
    let mut h = tracking();
    let tokens = h.worker.account.as_ref().unwrap().tokens.clone();
    list_workspaces(&mut h);
    h.machine.input_at(at("10:01:59"));
    h.command(at("10:02:01"), Command::SwitchToWorkspace { index: 1, at: at("10:02:00") });
    assert!(h.farewells.try_recv().is_err(), "no sign-out");
    let rows = h.worker.store.pending_rows(&h.owner(), 0, 10).unwrap();
    assert_eq!((rows[0].activity.ended_at, rows[0].workspace_id.as_str()), (at("10:02:00"), "w1"), "queued in the old workspace");

    let requests = h.control_requests();
    assert!(!requests.iter().any(|r| matches!(r, ControlRequest::SignIn(_))), "no new token, no password");
    let used = requests.iter().rev().find_map(|r| match r {
        ControlRequest::UseAccount(Some(account)) => Some(account.clone()),
        _ => None,
    });
    let used = used.expect("the new workspace is used");
    assert_eq!(used.workspace_id.as_deref(), Some("w2"));
    assert!(used.tokens == tokens, "the same token");
    assert!(requests.iter().any(|r| matches!(r, ControlRequest::RefreshProjects)));
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(Some(a)) if a.workspace_id.as_deref() == Some("w2"))));

    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!((saved.workspace_id.as_deref(), saved.workspace_name.as_str(), saved.project_id.as_deref()), (Some("w2"), "Globex", None));
    assert!(saved.projects.is_none());
    let snapshot = h.state.snapshot();
    assert_eq!((snapshot.status, snapshot.workspace.as_str()), (TrackerStatus::Connecting, "Globex"));
    assert!(snapshot.projects.is_empty() && snapshot.workspace_choices.is_empty());

    h.worker.on_control(ControlEvent::Projects(vec![project("p7", 5), project("p8", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::ChooseProject);
}

#[test]
fn switching_with_no_other_workspace_says_so() {
    let mut h = tracking();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme")])));
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
    h.worker.on_upload(UploadEvent::WorkspaceLost { owner: h.owner(), workspace_id: "w-other".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking, "rows of another workspace: not about this one");

    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into()));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::ChooseProject);
    assert!(snapshot.tracking_since.is_none());
    assert_eq!(snapshot.workspace_lost.as_deref(), Some("Acme"));
    assert_eq!(snapshot.workspace_note.as_deref(), Some("You no longer have access to Acme."));
    assert!(snapshot.notices.iter().any(|n| matches!(&n.kind, NoticeKind::WorkspaceLost { name } if name == "Acme")));
    assert!(snapshot.finding_workspaces);
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::MyWorkspaces)), "the picker is opened");
    let rows = h.worker.store.pending_rows(&h.owner(), 0, 10).unwrap();
    assert_eq!((rows[0].activity.ended_at, rows[0].workspace_id.as_str()), (at("10:04:00"), "w1"), "the work is kept, waiting");
    assert!(h.heartbeats().is_empty(), "no heartbeat without a workspace");

    // Another workspace is chosen: tracking can start there.
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w2", "Globex")])));
    h.command(at("10:05:00"), Command::SwitchToWorkspace { index: 0, at: at("10:05:00") });
    h.worker.on_control(ControlEvent::Projects(vec![project("p9", 5)]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!((snapshot.status, snapshot.workspace.as_str(), snapshot.workspace_lost.as_deref()), (TrackerStatus::Tracking, "Globex", None));
}

#[test]
fn tracking_active_follows_pause_and_resume_lifecycle() {
    let h_unsigned = harness_at(at("10:00:00"), None, false, None);
    assert!(!h_unsigned.machine.is_tracking_active(), "sensors must start inactive before sign-in");
    assert!(!h_unsigned.machine.is_input_running());
    assert!(!h_unsigned.machine.is_usage_running());

    let mut h = tracking();
    assert!(h.machine.is_tracking_active(), "sensors must activate when tracking project session");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 1);
    assert_eq!(h.machine.usage_starts(), 1);

    h.command(at("10:05:00"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert!(!h.machine.is_tracking_active(), "sensors must deactivate immediately when paused");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_stops(), 1);
    assert_eq!(h.machine.usage_stops(), 1);

    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    assert!(h.machine.is_tracking_active(), "sensors must reactivate upon resume");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 2);
    assert_eq!(h.machine.usage_starts(), 2);

    h.command(at("10:15:00"), Command::SignOut { at: at("10:15:00") });
    assert!(!h.machine.is_tracking_active(), "sensors must deactivate upon sign-out");
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

    // Machine locks
    h.worker.on_system(crate::session_events::SystemNotice {
        event: crate::session_events::SystemEvent::Locked,
        at: at("10:05:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(!h.machine.is_tracking_active(), "sensors must stop when machine is locked");
    assert!(!h.machine.is_input_running(), "input hooks must detach on lock");
    assert!(!h.machine.is_usage_running(), "window sampler must stop on lock");

    // Machine unlocks
    h.worker.on_system(crate::session_events::SystemNotice {
        event: crate::session_events::SystemEvent::Unlocked,
        at: at("10:06:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active(), "sensors must resume when machine unlocks");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_away_and_resumes_on_return() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    // Advance clock past idle threshold (default 10 min)
    h.machine.set(at("10:15:00"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(!h.machine.is_tracking_active(), "sensors must stop while user is away");
    assert!(!h.machine.is_input_running(), "input hooks must detach when away");
    assert!(!h.machine.is_usage_running(), "window sampler must stop when away");

    // User comes back: input at 10:16:00
    h.machine.input_at(at("10:16:00"));
    h.machine.set(at("10:16:01"));
    h.worker.on_tick();
    h.worker.publish();
    // Prompt is shown about time away; tracking resumed
    assert!(h.machine.is_tracking_active(), "sensors reactivate upon return");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_workspace_lost() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into()));
    h.worker.publish();
    assert_ne!(h.status(), TrackerStatus::Tracking);
    assert!(!h.machine.is_tracking_active(), "sensors must stop when workspace access is lost");
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

    // While quit is awaiting farewell and upload grace, assert that sensors were ALREADY halted!
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert!(!machine.is_tracking_active(), "sensors must stop BEFORE waiting for farewell/upload grace");
    assert!(!machine.is_input_running(), "input hooks must detach BEFORE waiting for farewell/upload grace");
    assert!(!machine.is_usage_running(), "usage sampler must stop BEFORE waiting for farewell/upload grace");
    assert_eq!(machine.input_stops(), 1);
    assert_eq!(machine.usage_stops(), 1);

    let h = quit_task.await.unwrap();
    assert!(!h.machine.is_tracking_active());
}

#[test]
fn worker_fails_closed_when_consent_is_absent() {
    let machine = FakeMachine::new(at("10:00:00"));
    let state = StateHandle::new();
    let (control, _, _) = ControlLink::detached();
    let (uploader, _) = UploaderHandle::detached();
    let store = Storage::open(":memory:", None).unwrap();
    let cfg = AppConfig::default(); // consent: LocalConsent::Absent
    let consent_store = Arc::new(MemoryConsentStore::new(LocalConsent::Absent));
    let mut worker = Worker::new(
        state,
        cfg,
        machine.clone(),
        Launch { autostart: false },
        Arc::new(|| {}),
        ServerClock::default(),
        store,
        control,
        uploader,
        None,
        consent_store,
    );
    worker.start(Some(saved(vec![project("p1", 5)], None)), None);
    worker.publish();
    assert!(!machine.is_tracking_active(), "absent consent must fail closed");
    assert!(!machine.is_input_running(), "input hooks must not start without consent");
    assert!(!machine.is_usage_running(), "sampler must not start without consent");
    assert_eq!(machine.input_starts(), 0);
    assert_eq!(machine.usage_starts(), 0);
}

#[test]
fn sensor_lifecycle_per_sensor_denied_policy() {
    let h = tracking();
    // Update session with keyboard allowed, mouse denied, active window denied
    let perms = CapturePermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        screenshot: false,
        webcam: false,
        running_programs: false,
        blur_screenshots: false,
    };
    h.machine.apply_sensor_policy(true, Some(&perms));
    assert!(h.machine.is_input_running(), "input hooks running for keyboard");
    assert!(!h.machine.is_usage_running(), "usage sampler must remain stopped when active_window is denied");
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

        h.command(t_pause, Command::Pause { resume_after: None, at: t_pause });
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
fn consent_2_legacy_config_absent() {
    let cfg: AppConfig = serde_json::from_str("{}").expect("deserialization from empty json must succeed");
    assert_eq!(cfg.consent, LocalConsent::Absent, "legacy config must fail closed to Absent");

    let nonexistent = PathBuf::from("dosi_nonexistent_consent_file_12345.json");
    let loaded = LocalConsent::load_from(&nonexistent);
    assert_eq!(loaded, LocalConsent::Absent, "missing consent file must fail closed to Absent");

    let h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        cfg.consent,
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
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(granular) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running(), "input hooks attached for keyboard");
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse not consented");
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
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    h.command(at("10:01:00"), Command::SelectProject { project_id: "p1".into(), at: at("10:01:00") });
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
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    h.command(at("10:02:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(h.machine.input_stops() >= 1);
    assert!(h.machine.usage_stops() >= 1);
}

#[test]
fn consent_6_policy_narrowing() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Narrowing by user consent (uncheck active_window)
    let mut user_narrowed: ConsentedPermissions = CapturePermissions::default().into();
    user_narrowed.active_window = false;
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(user_narrowed) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(!h.machine.is_usage_running(), "usage stopped after user narrowed consent");
    assert_eq!(h.machine.usage_stops(), 1);

    // Narrowing by server project policy (disallow keyboard and mouse)
    let restricted_project: Project = serde_json::from_value(serde_json::json!({
        "id": "p1", "title": "Project p1", "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false,
        "allowKeyboard": false, "allowMouse": false,
        "allowActiveWindow": false, "allowRunningPrograms": false,
        "blurScreenshots": true,
    })).unwrap();
    h.worker.on_control(ControlEvent::Projects(vec![restricted_project]));
    h.worker.publish();
    assert!(!h.machine.is_input_running(), "input stopped when workspace policy disallowed it");
    assert!(h.machine.input_stops() >= 1);
}

#[test]
fn consent_7_project_workspace_logout_transition() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // Logout immediately stops sensors
    h.command(at("10:05:00"), Command::SignOut { at: at("10:05:00") });
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());

    // Logging in when consent is revoked does not auto-start
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
    let dir = std::env::temp_dir().join(format!("dosi-test-consent-{}", uuid::Uuid::new_v4()));
    let path = dir.join("consent.json");

    let grant = LocalConsent::Granted(ConsentedPermissions {
        screenshot: true,
        webcam: false,
        keyboard: true,
        mouse: false,
        active_window: true,
        running_programs: false,
        blur_screenshots: true,
    });
    grant.save_to(&path).expect("save_to must succeed");

    let loaded = LocalConsent::load_from(&path);
    assert_eq!(loaded, grant, "loaded consent must match saved consent exactly");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn consent_9_corrupted_config_fail_closed() {
    let dir = std::env::temp_dir().join(format!("dosi-test-corrupt-{}", uuid::Uuid::new_v4()));
    let path = dir.join("consent.json");
    std::fs::create_dir_all(&dir).unwrap();

    // Empty file
    std::fs::write(&path, b"").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    // Corrupted JSON
    std::fs::write(&path, b"{ not valid json ::: ").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    // Invalid schema
    std::fs::write(&path, b"{\"Granted\": 99999}").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn consent_10_pause_resume_quit() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // Pause
    h.command(at("10:02:00"), Command::Pause { resume_after: None, at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert!(!h.machine.is_tracking_active());

    // Revoke consent while paused
    h.command(at("10:02:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    // Attempt to resume fails closed
    h.command(at("10:03:00"), Command::Resume { at: at("10:03:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());

    // Re-grant and resume
    h.command(at("10:03:30"), Command::UpdateConsent { consent: LocalConsent::Granted(CapturePermissions::default().into()) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // System lock stops tracking
    h.system(SystemEvent::Locked, at("10:04:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(!h.machine.is_tracking_active());

    // Revoke during lock
    h.command(at("10:04:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    h.system(SystemEvent::Unlocked, at("10:05:00"));
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must not restart after permission loss during lock");
}

#[test]
fn consent_11_no_duplicate_thread_start_on_repeated_user_actions() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let initial_input_starts = h.machine.input_starts();
    let initial_usage_starts = h.machine.usage_starts();

    // Repeated identical consent updates
    for i in 1..=5 {
        h.command(
            at(&format!("10:01:{:02}", i)),
            Command::UpdateConsent { consent: LocalConsent::Granted(CapturePermissions::default().into()) },
        );
    }
    assert_eq!(h.machine.input_starts(), initial_input_starts, "no duplicate input starts on repeated grant");
    assert_eq!(h.machine.usage_starts(), initial_usage_starts, "no duplicate usage starts on repeated grant");

    // Repeated resume and project selection
    for i in 1..=5 {
        h.command(at(&format!("10:02:{:02}", i)), Command::Resume { at: at(&format!("10:02:{:02}", i)) });
        h.command(
            at(&format!("10:03:{:02}", i)),
            Command::SelectProject { project_id: "p1".into(), at: at(&format!("10:03:{:02}", i)) },
        );
    }
    assert_eq!(h.machine.input_starts(), initial_input_starts, "repeated redundant commands do not duplicate threads");
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

    // Inject disk write fault
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
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(grant) });

    // Invariant: granting must not start sensors until safely accepted/persisted!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must NOT start on persistence failure");
    assert!(!h.machine.is_input_running(), "input hooks must NOT start");
    assert!(!h.machine.is_usage_running(), "usage sampler must NOT start");
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);

    // Error state surfaced actionably
    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_error.is_some(), "persistence error must be reported");
    assert!(snapshot.consent_unsynced, "consent_unsynced flag must be true");
    assert_eq!(snapshot.consent, LocalConsent::Absent, "grant not accepted into state on write failure");
}

#[test]
fn consent_13_revoke_persistence_failure_stops_sensors_and_reports_non_durable() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());

    // Inject disk write fault
    h.consent_store.set_fail_saves(true);

    // Revoke consent
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });

    // Invariant: revocation must stop sensors IMMEDIATELY even if disk fails!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must stop immediately on revoke");
    assert!(!h.machine.is_input_running(), "input hooks must be detached");
    assert!(!h.machine.is_usage_running(), "usage sampler must be stopped");
    assert!(h.machine.input_stops() >= 1);

    // State reports non-durable revocation clearly
    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_error.is_some(), "error message must describe non-durable disk write");
    let err_msg = snapshot.consent_error.as_ref().unwrap();
    assert!(err_msg.contains("non-durable across app restart"), "error describes restart durability limit honestly");
    assert!(snapshot.consent_unsynced, "consent_unsynced flag must be true");

    // Unintended resume must be prevented while unsynced/revoked
    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "resume must be blocked while consent is unsynced/revoked");
}

#[test]
fn consent_14_fake_worker_isolation_proves_no_disk_access() {
    // Proves that fake worker constructors and tests interact strictly with MemoryConsentStore.
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.consent_store.load_count(), 1, "loaded from MemoryConsentStore on start");
    assert_eq!(h.consent_store.save_count(), 0);

    let grant = ConsentedPermissions::default();
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(grant) });
    assert_eq!(h.consent_store.save_count(), 1, "saved to MemoryConsentStore on update");

    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.consent_store.save_count(), 2, "second save went to MemoryConsentStore");

    // No real disk consent file touched: MemoryConsentStore keeps state in memory mutex
    assert_eq!(h.consent_store.load(), LocalConsent::Revoked);
}

#[test]
fn consent_15_partial_grant_omitted_capabilities_strictly_default_deny() {
    // Prove that partial grant schema defaults ALL omitted capabilities strictly to false
    let json_only_mouse = r#"{"Granted": {"mouse": true}}"#;
    let consent: LocalConsent = serde_json::from_str(json_only_mouse).expect("partial json must deserialize");

    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    h.command(at("10:00:30"), Command::UpdateConsent { consent });
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Mouse was granted
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse granted");
    // All omitted capabilities are false:
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard omitted -> denied");
    assert!(!h.machine.is_usage_running(), "active window omitted -> denied");
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_16_narrowing_stops_withdrawn_sensors_before_blocked_save_completes() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );

    // Initial state: broad tracking active
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.is_usage_running());
    assert!(h.machine.is_screenshot_allowed());
    assert!(h.machine.is_webcam_allowed());

    // Setup injected blocking hook: inspect machine state WHILE save is in flight!
    let machine = h.machine.clone();
    let hook_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_hook(move |_target| {
        hook_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        // Invariant: Withdrawn permissions (keyboard, window, screen, webcam) MUST be stopped BEFORE save blocks!
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard MUST stop before save blocks");
        assert!(!machine.is_usage_running(), "usage sampler MUST stop before save blocks");
        assert!(!machine.is_screenshot_allowed(), "screenshot MUST be disallowed before save blocks");
        assert!(!machine.is_webcam_allowed(), "webcam MUST be disallowed before save blocks");

        // Retained permission (mouse) remains active without drop
        assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse remains active");
        assert!(machine.is_input_running(), "input running for mouse");
        assert_eq!(machine.captures(), 0, "no screen captures occurred");
        Ok(())
    });

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });

    // Ensure hook executed synchronously during save
    assert!(hook_called.load(std::sync::atomic::Ordering::SeqCst), "hook must have executed during save");
    assert_eq!(h.consent_store.save_count(), 1);

    // After save completed: state matches narrower grant
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());
    assert!(!h.machine.is_webcam_allowed());
}

#[test]
fn consent_17_mixed_narrowing_and_expansion_request_under_blocked_store() {
    // Start with mouse + keyboard, but NO webcam
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
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.is_webcam_allowed());

    // Request mixed change: withdraw keyboard, expand webcam
    let machine = h.machine.clone();
    let hook_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_hook(move |_target| {
        hook_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        // Withdrawn keyboard MUST be stopped
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "withdrawn keyboard stopped before save");
        // Newly requested webcam MUST NOT be active before storage confirms
        assert!(!machine.is_webcam_allowed(), "unpersisted expansion must NOT be active while save is blocked");
        // Retained mouse is active
        assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse active");
        Ok(())
    });

    let mixed = ConsentedPermissions {
        screenshot: false,
        webcam: true,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(mixed) });

    assert!(hook_called.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    // After persistence succeeded: webcam is now safely active!
    assert!(h.machine.is_webcam_allowed(), "webcam activated after successful persistence");
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn consent_18_blocked_save_failure_fails_closed_and_prevents_resume() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Save hook returns error to simulate write failure
    let machine = h.machine.clone();
    h.consent_store.set_save_hook(move |_target| {
        // Assert withdrawn permissions stopped before the write error
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
        anyhow::bail!("simulated disk I/O timeout during consent save")
    });

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });

    // Invariant: Save failure must establish fail-closed state!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "all sensors stopped on write failure");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());
    assert!(!h.machine.is_webcam_allowed());

    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_unsynced, "must report unsynced");
    assert_eq!(snapshot.consent, LocalConsent::Revoked, "fail-closed state does not falsely claim success");
    let err = snapshot.consent_error.as_ref().unwrap();
    assert!(err.contains("non-durable across app restart"), "honestly reports non-durable restart limit");

    // Resume attempt must be strictly blocked
    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "resume blocked while unsynced");
}

#[test]
fn consent_19_multithreaded_barrier_proves_withdrawn_sensors_stopped_while_store_blocked() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    let enter_barrier = Arc::new(std::sync::Barrier::new(2));
    let exit_barrier = Arc::new(std::sync::Barrier::new(2));

    let enter_clone = enter_barrier.clone();
    let exit_clone = exit_barrier.clone();
    h.consent_store.set_save_hook(move |_target| {
        enter_clone.wait(); // Signals test thread that save is currently blocked
        exit_clone.wait();  // Waits for test thread to complete its assertions
        Ok(())
    });

    let machine = h.machine.clone();
    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };

    // Run command on worker in a separate thread
    let worker_thread = std::thread::spawn(move || {
        h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });
        h
    });

    // Test coordinator thread: wait until store is BLOCKED in save()
    enter_barrier.wait();

    // At this EXACT moment, worker thread is literally blocked inside consent_store.save!
    // Assert withdrawn sensors are ALREADY stopped:
    assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard stopped while store blocked");
    assert!(!machine.is_usage_running(), "usage stopped while store blocked");
    assert!(!machine.is_screenshot_allowed(), "screenshot disallowed while store blocked");
    assert!(!machine.is_webcam_allowed(), "webcam disallowed while store blocked");
    assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse still running");
    assert_eq!(machine.captures(), 0, "no screen captures scheduled/taken");

    // Release worker thread to finish save
    exit_barrier.wait();

    let h = worker_thread.join().expect("worker thread must finish cleanly");
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.consent_store.save_count(), 1);
}

#[test]
fn consent_20_withdrawn_sensors_stopped_before_reconcile_record_blocks() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Deterministic channel-based synchronization asserting stopped drivers
    // WHILE finalization/record is actively executing in machine.snapshot!
    let (enter_tx, enter_rx) = std::sync::mpsc::channel::<()>();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
    let exit_rx = Mutex::new(exit_rx);

    h.machine.set_on_snapshot(move || {
        let _ = enter_tx.send(());
        let _ = exit_rx.lock().unwrap().recv();
    });

    let machine = h.machine.clone();

    let worker_thread = std::thread::spawn(move || {
        h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
        h
    });

    // Test coordinator: wait until worker reaches machine.snapshot during finalization/record
    enter_rx.recv().expect("worker thread must enter snapshot during finalization");

    // AT THIS EXACT MOMENT:
    // The worker is actively executing record() -> snapshot().
    // consent_store.save has NOT even been called yet.
    // Assert native sensor drivers were ALREADY stopped ahead of reconcile/record:
    assert!(!machine.is_tracking_active(), "tracking stopped before record/snapshot");
    assert!(!machine.is_input_running(), "input driver stopped before record/snapshot");
    assert!(!machine.is_usage_running(), "usage driver stopped before record/snapshot");
    assert_eq!(machine.captures(), 0, "no screen captures taken while record is blocked");
    assert_eq!(machine.webcam_captures(), 0, "no webcam captures taken while record is blocked");
    assert_eq!(machine.window_queries(), 0, "no window queries taken while record is blocked");
    assert_eq!(machine.process_queries(), 0, "no process queries taken while record is blocked");

    // Release worker thread cleanly
    drop(exit_tx);

    let h = worker_thread.join().expect("worker thread must finish cleanly");
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.consent_store.save_count(), 1);
    assert_eq!(h.consent_store.load(), LocalConsent::Revoked);
    assert_eq!(h.machine.snapshot_calls(), 1, "exactly one snapshot during finalization");

    assert_eq!(h.machine.captures(), 0, "final screen captures remain zero");
    assert_eq!(h.machine.webcam_captures(), 0, "final webcam captures remain zero");
    assert_eq!(h.machine.window_queries(), 0, "final window queries remain zero");
    assert_eq!(h.machine.process_queries(), 0, "final process queries remain zero");
}

#[test]
fn consent_21_held_blocks_do_not_acquire_withdrawn_screen_or_webcam_at_finalization() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 30)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Record user input at 10:03:00 during the active block
    h.machine.input_at(at("10:03:00"));

    // Tick past block boundary (10:10:01) with last input at 10:03:00.
    // Under 30-min idle threshold, the 10:00-10:10 block is held in stretch.held
    h.tick(at("10:10:01"));
    assert_eq!(h.queued().len(), 0, "first block is held awaiting user return");
    let captures_before_revoke = h.machine.captures();

    // At 10:12:00, user explicitly revokes consent while the block is held!
    h.command(at("10:12:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });

    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.snapshot_calls(), 2, "snapshots taken for both held and open block");

    // Assert actual acquisition call counts on fake machine:
    // Withdrawn sensors must NEVER perform fresh captures or queries at finalization
    assert_eq!(h.machine.captures(), captures_before_revoke, "no new screen captures taken at finalization");
    assert_eq!(h.machine.webcam_captures(), 0, "no webcam photos taken for held or open blocks");
    assert_eq!(h.machine.window_queries(), 0, "no foreground window sampling after withdrawal");
    assert_eq!(h.machine.process_queries(), 0, "no running programs listing after withdrawal");

    // Verify retained historical work:
    // Both the held block (10:00-10:10) and final sliver (10:10-10:12) are queued,
    // retaining already-authorized historical time and input counts without fresh capture data.
    let queued = h.queued();
    assert_eq!(queued.len(), 2, "both held block and closing sliver were finalized");

    let held_block = &queued[0];
    assert_eq!(held_block.started_at, at("10:00:00"));
    assert_eq!(held_block.ended_at, at("10:10:00"));
    assert_eq!(held_block.keyboard_hits, 1, "authorized key hits in held block preserved");
    assert_eq!(held_block.mouse_clicks, 1, "authorized clicks in held block preserved");
    assert_eq!(held_block.active_windows.len(), 1, "authorized usage slice preserved");
    assert_eq!(held_block.screenshot_png_base64, None, "no screenshot acquired on held block");
    assert_eq!(held_block.webcam_jpg_base64, None, "no webcam acquired on held block");
    assert!(held_block.running_programs.is_empty(), "no process sampling on held block");

    let open_block = &queued[1];
    assert_eq!(open_block.started_at, at("10:10:00"));
    assert_eq!(open_block.ended_at, at("10:12:00"));
    assert_eq!(open_block.screenshot_png_base64, None, "no screenshot acquired on open block");
    assert_eq!(open_block.webcam_jpg_base64, None, "no webcam acquired on open block");
    assert!(open_block.running_programs.is_empty(), "no process sampling on open block");
}

#[test]
fn consent_22_mixed_narrowing_stops_withdrawn_sensors_before_reconcile_record_blocks() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Record user input at 10:02:00
    h.machine.input_at(at("10:02:00"));

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };

    // User narrows permissions while tracking:
    h.command(at("10:05:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Native sensors for withdrawn capabilities are stopped immediately:
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard stopped");
    assert!(!h.machine.is_usage_running(), "usage sampler stopped");
    assert!(!h.machine.is_screenshot_allowed(), "screenshot disallowed");
    assert!(!h.machine.is_webcam_allowed(), "webcam disallowed");
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse still running");

    // Now tick past 10:10 to finalize the block:
    h.tick(at("10:10:01"));

    // Assert zero acquisitions for withdrawn capabilities:
    assert_eq!(h.machine.captures(), 0, "no screen captures taken at block boundary");
    assert_eq!(h.machine.webcam_captures(), 0, "no webcam photos taken at block boundary");
    assert_eq!(h.machine.window_queries(), 0, "no foreground window sampling at block boundary");
    assert_eq!(h.machine.process_queries(), 0, "no process queries at block boundary");

    let queued = h.queued();
    assert_eq!(queued.len(), 1, "interval block recorded");
    assert_eq!(queued[0].keyboard_hits, 0, "withdrawn keyboard count not populated");
    assert_eq!(queued[0].mouse_clicks, 1, "retained mouse clicks preserved");
    assert_eq!(queued[0].screenshot_png_base64, None, "no screenshot attached");
    assert_eq!(queued[0].webcam_jpg_base64, None, "no webcam attached");
    assert!(queued[0].running_programs.is_empty(), "no running programs attached");
}

