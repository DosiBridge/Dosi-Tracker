use std::sync::Mutex;

use chrono::FixedOffset;

use super::control::Farewell;
use super::*;
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
    let mut client = ApiClient::new(one_shot_server("400 Bad Request", refused), "a@b.c".into(), "x".into(), String::new());
    let error = client.login().await.expect_err("the server refuses");
    assert!(is_auth_rejection(&error));
    assert_eq!(friendly_login_error(&error), "Invalid username or password!");
}

#[tokio::test]
async fn a_server_error_is_not_a_rejected_sign_in() {
    let mut client = ApiClient::new(one_shot_server("503 Service Unavailable", ""), "a@b.c".into(), "x".into(), String::new());
    let error = client.login().await.expect_err("the server fails");
    assert!(!is_auth_rejection(&error));
}

#[tokio::test]
async fn an_unreachable_server_is_not_a_rejected_sign_in() {
    // Nothing listens on port 9 (discard) locally: a real connect error.
    let mut client = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into(), String::new());
    let error = client.login().await.expect_err("nothing is listening");
    assert!(!is_auth_rejection(&error));
}

#[test]
fn accounts_never_show_their_password() {
    let account = Account { server: "https://t.example".into(), workspace: "acme".into(), username: "a@acme.com".into(), password: "s3cr3t".into() };
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
        let measured = Measurements {
            key_presses: Vec::new(),
            clicks: Vec::new(),
            active_seconds: Vec::new(),
            injected: 0,
            input_available: true,
            slices: Vec::new(),
            window_now: None,
            running_programs: Vec::new(),
            screen,
            webcam_jpg_base64: None,
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

/// A saved sign-in whose project list was resolved an hour ago.
fn saved(projects: Vec<Project>, remembered: Option<&str>) -> Credentials {
    let mut saved = Credentials::new("acme".into(), "alice@acme.com".into(), "pw".into(), None);
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
}

fn harness_at(start: DateTime<Utc>, saved: Option<Credentials>, autostart: bool, credentials_file: Option<PathBuf>) -> Harness {
    let machine = FakeMachine::new(start);
    let state = StateHandle::new();
    let (control, control_rx, farewells) = ControlLink::detached();
    let (uploader, uploads) = UploaderHandle::detached();
    let store = Storage::open(":memory:", None).unwrap();
    let mut worker = Worker::new(
        state.clone(),
        AppConfig::default(),
        machine.clone(),
        Launch { autostart },
        Arc::new(|| {}),
        ServerClock::default(),
        store,
        control,
        uploader,
        credentials_file,
    );
    worker.start(saved, None);
    worker.publish();
    Harness { worker, machine, control: control_rx, farewells, uploads, state }
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
        storage::owner_key("acme", "alice@acme.com")
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
    let other = storage::owner_key("acme", "bob@acme.com");
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
    assert_eq!((snapshot.display_name.as_str(), snapshot.workspace.as_str()), ("alice@acme.com", "acme"), "prefilled");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(None))));
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::UseAccount(None))));
    assert!(h.heartbeats().is_empty(), "no heartbeats without a sign-in");

    // The password, entered again.
    h.command(at("10:10:00"), Command::SignIn { workspace: "acme".into(), username: "alice@acme.com".into(), password: "new".into(), server: None });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, AppConfig::default().api_base_url);
    h.worker.on_control(ControlEvent::SignedIn { account, projects: Ok(vec![project("p1", 5)]) });
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
        workspace: "acme".into(),
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("ftp://nope".into()),
    });
    assert!(h.state.snapshot().login_error.unwrap().contains("https://"), "an unusable address is refused at once");
    assert!(h.control_requests().is_empty());

    h.command(at("10:00:05"), Command::SignIn {
        workspace: "acme".into(),
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("tracker.acme.com/".into()),
    });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, "https://tracker.acme.com");
    h.worker.on_control(ControlEvent::SignedIn { account, projects: Ok(vec![project("a", 5), project("b", 5)]) });
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
    let account = Account {
        server: cfg.api_base_url.clone(),
        workspace: env("DOSI_E2E_WORKSPACE"),
        username: env("DOSI_E2E_USER"),
        password: env("DOSI_E2E_PASSWORD"),
    };
    let mut client = account.client(&ServerClock::default());
    client.login().await.expect("sign-in");
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
    store.enqueue(&activity, &owner).unwrap();
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
        workspace: env("DOSI_E2E_WORKSPACE"),
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
