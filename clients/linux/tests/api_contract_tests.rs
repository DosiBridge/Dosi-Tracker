//! Server-contract tests of the API client and the HTTP delivery transport.
//!
//! Verifies:
//! - Golden fixtures: the serialized activity (X11 fully measured, Wayland not
//!   measured, a kept idle block) and heartbeat bodies.
//! - Honest nulls: unmeasured counts are `null`, unmeasured productivity, idle
//!   seconds, timeline and captures are left out.
//! - The token request carries no `__tenant` and no workspace header, and sends
//!   the device id; workspace-scoped calls carry `X-Workspace-Id`.
//! - Park-and-continue end to end: a refused block is parked and the blocks
//!   after it are uploaded in the same pass.
//! - 413 strips the captures and resends at once; a second refusal parks.
//! - Terminal answers (DeviceRevoked, NotWorkspaceMember), project re-checks,
//!   subscription/clock retries, and the local integrity check.
//!
//! Everything runs against canned loopback servers; nothing leaves the host.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use dosi_tracker_linux::agent::model::{
    Activity, ActivityMinute, EndReason, Heartbeat, PresenceStatus, WindowInfo, PLATFORM,
};
use dosi_tracker_linux::api::{
    ApiClient, HttpDeliveryTransport, ServerClock, TokenCell, TransportConfig,
    INTEGRITY_FAILURE_REASON, WORKSPACE_HEADER,
};
use dosi_tracker_linux::outbox::{
    CaptureKind, Outbox, OutboxLimits, OutboxScope, PendingActivity, PendingCapture, RowState,
};
use dosi_tracker_linux::sync::{
    RetryKind, ScopeBlock, SyncPassConfig, SyncPassCoordinator, SyncStopReason,
};
use tempfile::tempdir;

const DEVICE: &str = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11";
const PROJECT: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
const WORKSPACE: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const ACCOUNT: &str = "alice@acme.test";
const KEY: [u8; 32] = [7u8; 32];

// --- Golden fixtures --------------------------------------------------------

fn fixture(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/contract")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn at(hour: u32, minute: u32, second: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 5, hour, minute, second)
        .unwrap()
}

fn base_block(id: &str, start: chrono::DateTime<Utc>, end: chrono::DateTime<Utc>) -> Activity {
    let mut activity = Activity::new(PROJECT, start, end);
    activity.client_activity_id = id.to_string();
    activity.platform = Some(PLATFORM.into());
    activity.agent_version = Some("0.1.0".into());
    activity.device_id = Some(DEVICE.into());
    activity.device_name = Some("workstation-1".into());
    activity.payload_signature = Some("c2lnbmF0dXJl".into());
    activity
}

fn program(name: &str) -> WindowInfo {
    WindowInfo {
        app_name: name.into(),
        ..Default::default()
    }
}

fn minute(
    index: u32,
    keys: u64,
    clicks: u64,
    app: Option<&str>,
    active_seconds: u32,
) -> ActivityMinute {
    ActivityMinute {
        minute: index,
        keyboard_hits: Some(keys),
        mouse_clicks: Some(clicks),
        active: active_seconds > 0,
        app_name: app.map(str::to_string),
        active_seconds: Some(active_seconds),
    }
}

fn x11_full() -> Activity {
    let mut a = base_block(
        "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11",
        at(10, 0, 0),
        at(10, 3, 0),
    );
    a.description = Some("Release notes".into());
    a.productivity = Some(67);
    a.mouse_clicks = Some(42);
    a.keyboard_hits = Some(310);
    a.active_windows = vec![
        WindowInfo {
            app_name: "code".into(),
            window_title: "main.rs - dosi-tracker".into(),
            seconds: Some(140),
            keyboard_hits: Some(300),
            mouse_clicks: Some(30),
        },
        WindowInfo {
            app_name: "firefox".into(),
            window_title: "Docs".into(),
            seconds: Some(40),
            keyboard_hits: Some(10),
            mouse_clicks: Some(12),
        },
    ];
    a.running_programs = vec![program("code"), program("firefox")];
    a.timeline = vec![
        minute(0, 200, 20, Some("code"), 55),
        minute(1, 110, 22, Some("firefox"), 41),
        minute(2, 0, 0, None, 0),
    ];
    a.screenshot_png_base64 = Some("/9j/4AAQSkZJRgABAQ==".into());
    a.screenshot_thumb_jpg_base64 = Some("/9j/2wBDAAgGBg==".into());
    a.os_version = Some("Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)".into());
    a.time_zone = Some("Asia/Dhaka".into());
    a.utc_offset_minutes = Some(360);
    a.idle_seconds = Some(60);
    a.end_reason = Some(EndReason::Interval);
    a.sent_at = Some(at(10, 3, 2));
    a.screenshot_captured_at = Some(at(10, 1, 30));
    a.screenshot_blurred = Some(false);
    a.injected_input_events = Some(4);
    a
}

/// Wayland: nothing about input or windows can be measured, and the portal
/// capture was not taken. Counts stay `null`; the rest is left out.
fn wayland_not_measured() -> Activity {
    let mut a = base_block(
        "1c8b6d44-3e26-4e3f-96d4-4b7e0e5a1c22",
        at(10, 0, 0),
        at(10, 10, 0),
    );
    a.running_programs = vec![program("gnome-text-editor"), program("firefox")];
    a.os_version =
        Some("Fedora Linux 43 (Workstation Edition) (Linux 6.17.1-300.fc43.x86_64)".into());
    a.time_zone = Some("Europe/Berlin".into());
    a.utc_offset_minutes = Some(120);
    a.end_reason = Some(EndReason::Lock);
    a.sent_at = Some(at(10, 10, 1));
    a
}

/// A block the user kept after the away prompt: measured, all zero.
fn kept_idle() -> Activity {
    let mut a = base_block(
        "2d7a5c33-4d15-4d2e-85c3-3a6d9d4b0b33",
        at(11, 0, 0),
        at(11, 3, 0),
    );
    a.description = Some("Whiteboard session".into());
    a.productivity = Some(0);
    a.mouse_clicks = Some(0);
    a.keyboard_hits = Some(0);
    a.timeline = (0..3).map(|i| minute(i, 0, 0, None, 0)).collect();
    a.os_version = Some("Ubuntu 22.04.5 LTS (Linux 6.8.0-45-generic)".into());
    a.time_zone = Some("Asia/Dhaka".into());
    a.utc_offset_minutes = Some(360);
    a.idle_seconds = Some(180);
    a.end_reason = Some(EndReason::Idle);
    a.sent_at = Some(at(11, 3, 5));
    a.injected_input_events = Some(0);
    a
}

fn tracking_heartbeat() -> Heartbeat {
    Heartbeat {
        device_id: DEVICE.into(),
        device_name: Some("workstation-1".into()),
        platform: Some(PLATFORM.into()),
        os_version: Some("Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)".into()),
        agent_version: Some("0.1.0".into()),
        status: PresenceStatus::Tracking,
        project_id: Some(PROJECT.into()),
        status_since: Some(at(10, 0, 0)),
        current_app: Some("code".into()),
        time_zone: Some("Asia/Dhaka".into()),
        utc_offset_minutes: Some(360),
        sent_at: Some(at(10, 10, 2)),
    }
}

#[test]
fn serialized_bodies_match_the_golden_fixtures() {
    for (name, value) in [
        (
            "activity_x11_full.json",
            serde_json::to_value(x11_full()).unwrap(),
        ),
        (
            "activity_wayland_not_measured.json",
            serde_json::to_value(wayland_not_measured()).unwrap(),
        ),
        (
            "activity_kept_idle.json",
            serde_json::to_value(kept_idle()).unwrap(),
        ),
        (
            "heartbeat_tracking.json",
            serde_json::to_value(tracking_heartbeat()).unwrap(),
        ),
    ] {
        assert_eq!(
            value,
            fixture(name),
            "{name} drifted from its golden fixture"
        );
    }
}

#[test]
fn the_golden_fixtures_round_trip_through_the_model() {
    for name in [
        "activity_x11_full.json",
        "activity_wayland_not_measured.json",
        "activity_kept_idle.json",
    ] {
        let parsed: Activity = serde_json::from_value(fixture(name)).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            fixture(name),
            "{name}"
        );
    }
}

#[test]
fn the_wayland_fixture_sends_nothing_it_did_not_measure() {
    let wayland = fixture("activity_wayland_not_measured.json");
    let object = wayland.as_object().unwrap();
    for absent in [
        "productivity",
        "idleSeconds",
        "timeline",
        "injectedInputEvents",
        "screenshotPngBase64",
        "screenshotThumbJpgBase64",
        "webcamJpgBase64",
        "screenshotCapturedAt",
        "screenshotBlurred",
    ] {
        assert!(!object.contains_key(absent), "{absent} must be left out");
    }
    // Counts are nullable in the contract: null = not measured, never 0.
    for count in ["keyboardHits", "mouseClicks"] {
        assert!(object.contains_key(count), "{count} is sent");
        assert!(object[count].is_null(), "{count} must be null, never 0");
    }
    assert_eq!(object["activeWindows"], serde_json::json!([]));

    // A kept idle block is measured: its zeros are real.
    let kept = fixture("activity_kept_idle.json");
    assert_eq!(kept["keyboardHits"], 0);
    assert_eq!(kept["mouseClicks"], 0);
    assert_eq!(kept["productivity"], 0);
}

// --- Canned loopback server --------------------------------------------------

fn http_resp(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let extra: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
}

fn token_ok() -> String {
    http_resp(
        "200 OK",
        &[],
        r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600,"refresh_token":"r1"}"#,
    )
}

fn ok() -> String {
    http_resp("200 OK", &[], "{}")
}

fn abp(status: &str, code: &str, message: &str) -> String {
    http_resp(
        status,
        &[],
        &format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#),
    )
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&data);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&data).into_owned()
}

/// Serve `responses` in order, one per connection; every request is passed on.
fn canned_server(responses: Vec<String>) -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = tx.send(read_request(&mut stream));
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (url, rx)
}

fn header_of<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let head = &request[..request.find("\r\n\r\n").unwrap_or(request.len())];
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn json_body(request: &str) -> serde_json::Value {
    serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap()
}

fn requests(rx: &Receiver<String>) -> Vec<String> {
    // The server thread may still be writing the last answer.
    std::thread::sleep(Duration::from_millis(50));
    rx.try_iter().collect()
}

// --- Token and header rules --------------------------------------------------

#[tokio::test]
async fn the_token_request_has_no_tenant_and_no_workspace_header() {
    let (url, rx) = canned_server(vec![token_ok(), http_resp("200 OK", &[], "[]")]);
    let mut api = ApiClient::new(&url, ACCOUNT.into(), "pw".into())
        .with_workspace(Some(WORKSPACE.into()))
        .with_device_id(Some(DEVICE.into()));
    api.login().await.unwrap();
    api.my_projects().await.unwrap();

    let all = requests(&rx);
    let token = &all[0];
    assert!(token.starts_with("POST /connect/token "), "{token}");
    assert!(!token.to_lowercase().contains("__tenant"), "{token}");
    assert_eq!(header_of(token, WORKSPACE_HEADER), None);
    assert!(token.contains(&format!("device_id={DEVICE}")), "{token}");
    assert_eq!(header_of(&all[1], WORKSPACE_HEADER), Some(WORKSPACE));
}

// --- Delivery through the sync pass -----------------------------------------

struct Queue {
    _dir: tempfile::TempDir,
    outbox: Outbox,
    scope: OutboxScope,
}

fn queue() -> Queue {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    Queue {
        _dir: dir,
        outbox,
        scope: OutboxScope::new(ACCOUNT, WORKSPACE).unwrap(),
    }
}

fn block(n: u32) -> Activity {
    let start = at(9, n * 10, 0);
    let mut a = Activity::new(PROJECT, start, start + chrono::Duration::minutes(10));
    a.client_activity_id = uuid::Uuid::new_v4().to_string();
    a.platform = Some(PLATFORM.into());
    a.device_id = Some(DEVICE.into());
    a.description = Some(format!("block {n}"));
    a
}

fn enqueue(q: &Queue, activity: &Activity, captures: Vec<PendingCapture>) -> String {
    let pending = PendingActivity::from_activity(activity, captures).unwrap();
    q.outbox.enqueue_activity(&q.scope, &pending, &KEY).unwrap();
    activity.client_activity_id.clone()
}

fn captures() -> Vec<PendingCapture> {
    vec![
        PendingCapture {
            kind: CaptureKind::Screen,
            jpeg: vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3],
            captured_at: at(9, 5, 0),
            blurred: true,
        },
        PendingCapture {
            kind: CaptureKind::Thumb,
            jpeg: vec![0xFF, 0xD8, 0xFF, 0xDB, 4],
            captured_at: at(9, 5, 0),
            blurred: true,
        },
    ]
}

fn transport(url: &str) -> HttpDeliveryTransport {
    HttpDeliveryTransport::new_shared(TransportConfig {
        server: url.to_string(),
        account: ACCOUNT.into(),
        password: Some(zeroize::Zeroizing::new("pw".to_string())),
        tokens: TokenCell::default(),
        workspace_id: WORKSPACE.into(),
        device_id: Some(DEVICE.into()),
        signing_key: Some(KEY),
        clock: ServerClock::default(),
        allow_insecure_http: false,
    })
    .unwrap()
}

fn row_state(q: &Queue, id: &str) -> Option<(RowState, Option<String>)> {
    q.outbox
        .recent_rows(ACCOUNT, 200)
        .unwrap()
        .into_iter()
        .find(|row| row.client_activity_id == id)
        .map(|row| (row.state, row.parked_reason))
}

#[test]
fn a_refused_block_is_parked_and_the_rest_upload_in_the_same_pass() {
    let q = queue();
    let refused = enqueue(&q, &block(0), vec![]);
    let second = enqueue(&q, &block(1), captures());
    let third = enqueue(&q, &block(2), vec![]);

    let (url, rx) = canned_server(vec![
        token_ok(),
        abp(
            "409 Conflict",
            "Tracker:ActivityOverlaps",
            "Overlaps another block.",
        ),
        ok(),
        ok(),
    ]);
    let mut transport = transport(&url);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();

    assert_eq!(report.parked.len(), 1);
    assert_eq!(report.parked[0].client_activity_id, refused);
    assert_eq!(report.parked[0].reason, "Overlaps another block.");
    assert_eq!(
        report.persisted_acknowledged,
        vec![second.clone(), third.clone()]
    );
    assert!(report.drained, "{:?}", report.stop_reason);
    assert_eq!(report.stop_reason, None);
    assert_eq!(
        row_state(&q, &refused),
        Some((RowState::Parked, Some("Overlaps another block.".into())))
    );
    assert_eq!(q.outbox.pending_count(&q.scope).unwrap(), 0);

    let all = requests(&rx);
    assert_eq!(all.len(), 4);
    for upload in &all[1..] {
        assert!(upload.starts_with("POST /api/app/activity "), "{upload}");
        assert_eq!(header_of(upload, WORKSPACE_HEADER), Some(WORKSPACE));
        let body = json_body(upload);
        assert!(body["sentAt"].is_string(), "sentAt per attempt");
        assert!(
            body["payloadSignature"].is_string(),
            "the row's v2 signature"
        );
        assert!(body["keyboardHits"].is_null() && body["mouseClicks"].is_null());
        assert!(body.get("timeline").is_none());
    }
    // The second block carried its captures, hydrated as base64.
    let with_captures = json_body(&all[2]);
    assert_eq!(with_captures["clientActivityId"], second);
    assert_eq!(with_captures["screenshotPngBase64"], "/9j/4AECAw==");
    assert_eq!(with_captures["screenshotThumbJpgBase64"], "/9j/2wQ=");
    assert_eq!(with_captures["screenshotBlurred"], true);
    assert_eq!(
        with_captures["screenshotCapturedAt"],
        "2026-10-05T09:05:00Z"
    );
}

#[test]
fn a_413_strips_the_captures_and_resends_at_once() {
    let q = queue();
    let id = enqueue(&q, &block(0), captures());
    let (url, rx) = canned_server(vec![
        token_ok(),
        http_resp("413 Payload Too Large", &[], ""),
        ok(),
    ]);
    let mut transport = transport(&url);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();

    assert_eq!(report.stripped, vec![id.clone()]);
    assert_eq!(report.persisted_acknowledged, vec![id.clone()]);
    assert!(report.parked.is_empty());
    assert_eq!(q.outbox.count(None).unwrap(), 0);

    let all = requests(&rx);
    assert_eq!(all.len(), 3);
    assert!(json_body(&all[1])["screenshotPngBase64"].is_string());
    let resent = json_body(&all[2]);
    assert_eq!(resent["clientActivityId"], id);
    for key in [
        "screenshotPngBase64",
        "screenshotThumbJpgBase64",
        "screenshotCapturedAt",
        "screenshotBlurred",
    ] {
        assert!(resent.get(key).is_none(), "{key} stripped");
    }
}

#[test]
fn a_second_capture_refusal_parks_the_block() {
    let q = queue();
    let id = enqueue(&q, &block(0), captures());
    let next = enqueue(&q, &block(1), vec![]);
    let (url, _rx) = canned_server(vec![
        token_ok(),
        http_resp("413 Payload Too Large", &[], ""),
        abp("400 Bad Request", "Tracker:InvalidCaptureData", "Bad image"),
        ok(),
    ]);
    let mut transport = transport(&url);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();
    assert_eq!(report.stripped, vec![id.clone()]);
    assert_eq!(report.parked[0].client_activity_id, id);
    assert_eq!(report.persisted_acknowledged, vec![next]);
    assert_eq!(row_state(&q, &id).unwrap().0, RowState::Parked);
}

#[test]
fn a_revoked_device_parks_its_block_and_blocks_the_scope() {
    let q = queue();
    let id = enqueue(&q, &block(0), vec![]);
    let kept = enqueue(&q, &block(1), vec![]);
    let (url, _rx) = canned_server(vec![
        token_ok(),
        abp(
            "403 Forbidden",
            "Tracker:DeviceRevoked",
            "This device was revoked.",
        ),
    ]);
    let mut transport = transport(&url);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();
    match report.stop_reason {
        Some(SyncStopReason::ScopeBlocked {
            ref client_activity_id,
            block: ScopeBlock::DeviceRevoked,
            ref reason,
        }) => {
            assert_eq!(client_activity_id, &id);
            assert!(reason.contains("revoked"), "{reason}");
        }
        ref other => panic!("expected ScopeBlocked(DeviceRevoked), got {other:?}"),
    }
    assert_eq!(row_state(&q, &id).unwrap().0, RowState::Parked);
    assert_eq!(row_state(&q, &kept).unwrap().0, RowState::Pending);

    // The uploader parks the scope's other rows.
    assert_eq!(q.outbox.park_scope(&q.scope, "device revoked").unwrap(), 1);
    assert_eq!(q.outbox.pending_count(&q.scope).unwrap(), 0);
}

#[test]
fn lost_workspace_access_keeps_the_block_and_blocks_the_scope() {
    for (status, code) in [
        ("403 Forbidden", "Tracker:NotWorkspaceMember"),
        ("403 Forbidden", "Tracker:WorkspaceArchived"),
        ("400 Bad Request", "Tracker:WorkspaceRequired"),
    ] {
        let q = queue();
        let id = enqueue(&q, &block(0), vec![]);
        let (url, _rx) = canned_server(vec![token_ok(), abp(status, code, "No access")]);
        let mut transport = transport(&url);
        let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
            .run_pass(&q.scope, &mut transport)
            .unwrap();
        assert!(
            matches!(
                report.stop_reason,
                Some(SyncStopReason::ScopeBlocked {
                    block: ScopeBlock::WorkspaceAccessLost,
                    ..
                })
            ),
            "{code}: {:?}",
            report.stop_reason
        );
        assert_eq!(row_state(&q, &id).unwrap().0, RowState::Pending, "{code}");
        assert!(report.parked.is_empty());
    }
}

#[test]
fn an_unavailable_project_parks_only_when_it_is_really_gone() {
    let gone = r#"[{"id":"11111111-1111-1111-1111-111111111111","title":"Other","intervalMinutes":10,
        "allowScreenshot":true,"allowWebcam":false,"allowKeyboard":true,"allowMouse":true,
        "allowActiveWindow":true,"allowRunningPrograms":true}]"#;
    let still_there = format!(
        r#"[{{"id":"{PROJECT}","title":"Website","intervalMinutes":10,"allowScreenshot":true,
        "allowWebcam":false,"allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,
        "allowRunningPrograms":true}}]"#
    );

    let q = queue();
    let id = enqueue(&q, &block(0), vec![]);
    let (url, rx) = canned_server(vec![
        token_ok(),
        abp("403 Forbidden", "Tracker:NotProjectMember", "Not a member"),
        http_resp("200 OK", &[], gone),
    ]);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport(&url))
        .unwrap();
    assert_eq!(report.parked[0].client_activity_id, id);
    assert!(report.parked[0].reason.contains("Tracker:NotProjectMember"));
    let all = requests(&rx);
    assert!(all[2].starts_with("GET /api/app/project/my-projects "));
    assert_eq!(header_of(&all[2], WORKSPACE_HEADER), Some(WORKSPACE));

    let q = queue();
    let id = enqueue(&q, &block(0), vec![]);
    let (url, _rx) = canned_server(vec![
        token_ok(),
        abp("409 Conflict", "Tracker:ProjectArchived", "Archived"),
        http_resp("200 OK", &[], &still_there),
    ]);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport(&url))
        .unwrap();
    assert!(report.parked.is_empty());
    assert!(matches!(
        report.stop_reason,
        Some(SyncStopReason::RetryLater {
            kind: RetryKind::Transient,
            ..
        })
    ));
    assert_eq!(row_state(&q, &id).unwrap().0, RowState::Pending);
}

#[test]
fn subscription_clock_and_throttling_answers_retry_later_by_kind() {
    for (response, kind, wait) in [
        (
            abp(
                "402 Payment Required",
                "Tracker:SubscriptionInactive",
                "Inactive",
            ),
            RetryKind::Subscription,
            None,
        ),
        (
            abp("425 Too Early", "Tracker:ActivityInFuture", "In the future"),
            RetryKind::ClockAhead,
            None,
        ),
        (
            http_resp("429 Too Many Requests", &[("Retry-After", "90")], ""),
            RetryKind::Transient,
            Some(Duration::from_secs(90)),
        ),
    ] {
        let q = queue();
        let id = enqueue(&q, &block(0), vec![]);
        let (url, _rx) = canned_server(vec![token_ok(), response]);
        let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
            .run_pass(&q.scope, &mut transport(&url))
            .unwrap();
        match report.stop_reason {
            Some(SyncStopReason::RetryLater {
                kind: got,
                retry_after,
                ..
            }) => {
                assert_eq!(got, kind);
                assert_eq!(retry_after, wait);
            }
            ref other => panic!("expected RetryLater, got {other:?}"),
        }
        let pending = q.outbox.peek_ready(&q.scope, 10).unwrap();
        assert_eq!(pending[0].client_activity_id, id);
        assert_eq!(pending[0].retry_count, 1, "the attempt is recorded");
    }
}

#[test]
fn a_tampered_row_is_parked_without_contacting_the_server() {
    let q = queue();
    let id = enqueue(&q, &block(0), captures());
    let good = enqueue(&q, &block(1), vec![]);
    {
        let conn = rusqlite::Connection::open(q.outbox.path()).unwrap();
        conn.execute(
            "UPDATE outbox_entries SET payload_json = replace(payload_json, 'block 0', 'block 9')
             WHERE client_activity_id = ?1",
            [&id],
        )
        .unwrap();
    }
    let (url, rx) = canned_server(vec![token_ok(), ok()]);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport(&url))
        .unwrap();
    assert_eq!(report.parked[0].client_activity_id, id);
    assert_eq!(report.parked[0].reason, INTEGRITY_FAILURE_REASON);
    assert_eq!(report.persisted_acknowledged, vec![good.clone()]);
    let all = requests(&rx);
    assert_eq!(all.len(), 2, "only the good block was sent");
    assert_eq!(json_body(&all[1])["clientActivityId"], good);
}

#[test]
fn a_swapped_capture_fails_the_integrity_check() {
    let q = queue();
    let id = enqueue(&q, &block(0), captures());
    {
        let conn = rusqlite::Connection::open(q.outbox.path()).unwrap();
        conn.execute(
            "UPDATE outbox_captures SET jpeg = X'FFD8FFE0090909' WHERE kind = 'screen'",
            [],
        )
        .unwrap();
    }
    let (url, rx) = canned_server(vec![]);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport(&url))
        .unwrap();
    assert_eq!(report.parked[0].client_activity_id, id);
    assert_eq!(report.parked[0].reason, INTEGRITY_FAILURE_REASON);
    assert!(requests(&rx).is_empty());
}

#[test]
fn a_stripped_row_still_verifies_and_uploads() {
    let q = queue();
    let id = enqueue(&q, &block(0), captures());
    assert!(q.outbox.strip_captures(&q.scope, &id).unwrap());
    let (url, rx) = canned_server(vec![token_ok(), ok()]);
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport(&url))
        .unwrap();
    assert_eq!(report.persisted_acknowledged, vec![id]);
    let body = json_body(&requests(&rx)[1]);
    assert!(body.get("screenshotPngBase64").is_none());
}

#[test]
fn refused_credentials_stop_the_pass_as_sign_in_expired() {
    let q = queue();
    enqueue(&q, &block(0), vec![]);
    let (url, _rx) = canned_server(vec![http_resp(
        "400 Bad Request",
        &[],
        r#"{"error":"invalid_grant","error_description":"The refresh token is no longer valid."}"#,
    )]);
    let mut transport = HttpDeliveryTransport::new_shared(TransportConfig {
        server: url,
        account: ACCOUNT.into(),
        password: None,
        tokens: TokenCell::with_refresh_token(Some("stale".into())),
        workspace_id: WORKSPACE.into(),
        device_id: Some(DEVICE.into()),
        signing_key: Some(KEY),
        clock: ServerClock::default(),
        allow_insecure_http: false,
    })
    .unwrap();
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();
    assert!(matches!(
        report.stop_reason,
        Some(SyncStopReason::AuthRequired { .. })
    ));
    assert_eq!(q.outbox.pending_count(&q.scope).unwrap(), 1);
}

#[test]
fn rotated_refresh_tokens_reach_the_listener_through_the_transport() {
    let q = queue();
    enqueue(&q, &block(0), vec![]);
    let (url, _rx) = canned_server(vec![
        http_resp(
            "200 OK",
            &[],
            r#"{"access_token":"t","expires_in":3600,"refresh_token":"rotated-2"}"#,
        ),
        ok(),
    ]);
    let tokens = TokenCell::with_refresh_token(Some("r1".into()));
    let (tx, rotated) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    tokens.set_rotation_listener(std::sync::Arc::new(move |token| {
        let _ = tx.lock().unwrap().send(token);
    }));
    let mut transport = HttpDeliveryTransport::new_shared(TransportConfig {
        server: url,
        account: ACCOUNT.into(),
        password: None,
        tokens: tokens.clone(),
        workspace_id: WORKSPACE.into(),
        device_id: Some(DEVICE.into()),
        signing_key: Some(KEY),
        clock: ServerClock::default(),
        allow_insecure_http: false,
    })
    .unwrap();
    let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
        .run_pass(&q.scope, &mut transport)
        .unwrap();
    assert_eq!(report.persisted_acknowledged.len(), 1);
    assert_eq!(rotated.try_recv().unwrap(), "rotated-2");
    assert_eq!(tokens.refresh_token().as_deref(), Some("rotated-2"));
}

#[test]
fn every_server_answer_class_maps_to_one_documented_outcome() {
    // A compact view of the design 8.4 table through the real transport.
    let cases: Vec<(String, &str)> = vec![
        (http_resp("500 Internal Server Error", &[], ""), "retry"),
        (http_resp("404 Not Found", &[], ""), "retry"),
        (
            abp(
                "422 Unprocessable Entity",
                "Tracker:ActivityTooOld",
                "Too old",
            ),
            "park",
        ),
        (
            abp(
                "422 Unprocessable Entity",
                "Tracker:ActivityTooLong",
                "Too long",
            ),
            "park",
        ),
        (
            abp(
                "409 Conflict",
                "Tracker:ActivityClientIdConflict",
                "Conflict",
            ),
            "park",
        ),
        (
            abp("422 Unprocessable Entity", "Tracker:Unknown", "?"),
            "retry",
        ),
    ];
    let mut seen = HashMap::new();
    for (response, expected) in cases {
        let q = queue();
        enqueue(&q, &block(0), vec![]);
        let (url, _rx) = canned_server(vec![token_ok(), response.clone()]);
        let report = SyncPassCoordinator::new(&q.outbox, SyncPassConfig::default())
            .run_pass(&q.scope, &mut transport(&url))
            .unwrap();
        let got = if !report.parked.is_empty() {
            "park"
        } else if matches!(report.stop_reason, Some(SyncStopReason::RetryLater { .. })) {
            "retry"
        } else {
            "other"
        };
        assert_eq!(got, expected, "{response}");
        *seen.entry(got).or_insert(0) += 1;
    }
    assert_eq!(seen["park"], 3);
}
