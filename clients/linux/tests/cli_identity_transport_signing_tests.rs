//! End-to-end identity, transport and signing (v1 contract).
//!
//! Enforces:
//! - CLI login (password on stdin, through the daemon) -> status -> logout,
//!   against a stateful fake backend: no `__tenant`, the install's device id
//!   on token requests, no password or token in any output.
//! - Fail-closed status: with the agent down `status` exits 3, prints local
//!   facts and never creates files or directories.
//! - Queue depth from the read-only inspector.
//! - The credentials the sign-in persisted (file store: tokens and the device
//!   signing key, never the password) drive a v2 transport: signed delivery,
//!   ack deletion, and transient failures (HTTP 503, an interrupted socket)
//!   that keep the row and count the attempt, without touching other scopes.
//! - A refused sign-in stores nothing.
//! - Transport scope binding (library level, unchanged).

#![cfg(target_os = "linux")]

mod support;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use tempfile::tempdir;

use dosi_tracker_linux::agent::model::{Activity, PLATFORM};
use dosi_tracker_linux::api::{
    verify_v2_entry, ApiClient, HttpDeliveryTransport, ServerClock, TokenCell, TransportConfig,
};
use dosi_tracker_linux::credentials::{
    CredentialStore, Credentials, FileCredentialStore, InMemoryCredentialStore,
};
use dosi_tracker_linux::outbox::{
    ActivityEnvelope, Outbox, OutboxLimits, OutboxScope, PendingActivity,
};
use dosi_tracker_linux::sync::{SyncPassConfig, SyncPassCoordinator, SyncStopReason};
use support::agent::{parse, Agent};
use support::fake_backend::{ActivityMode, FakeBackend, PROJECT_WEBSITE, WS_ACME};

const EMAIL: &str = "tester@dosi.test";
const PASSWORD: &str = "super-secret-password";

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
        r#"{"access_token":"mock-token-xyz","token_type":"Bearer","expires_in":3600,"refresh_token":"mock-refresh-xyz"}"#,
    )
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
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

/// Answers the token request, then reads the upload's headers and drops the
/// connection without an answer.
fn interrupting_server() -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let req = read_request(&mut stream);
            let _ = tx.send(req);
            let _ = stream.write_all(token_ok().as_bytes());
            let _ = stream.flush();
        }
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut read_bytes = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                read_bytes.extend_from_slice(&buf[..n]);
                if read_bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&read_bytes).into_owned());
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    (url, rx, handle)
}

fn counting_server_loop(
    responses: Vec<String>,
) -> (
    String,
    std::sync::mpsc::Receiver<String>,
    Arc<AtomicUsize>,
    std::sync::mpsc::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let _ = listener.set_nonblocking(true);
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();
    std::thread::spawn(move || {
        let mut resp_iter = responses.into_iter();
        while shutdown_rx.try_recv().is_err() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.set_nonblocking(false);
                    let req = read_request(&mut stream);
                    let _ = tx.send(req);
                    let resp = resp_iter.next().unwrap_or_else(token_ok);
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (url, rx, counter, shutdown_tx)
}

fn sample_envelope(client_activity_id: &str) -> ActivityEnvelope {
    let now = Utc::now();
    ActivityEnvelope {
        format_version: 1,
        client_activity_id: client_activity_id.to_string(),
        project_id: Some("proj-uuid-100".to_string()),
        started_at: now - chrono::Duration::minutes(5),
        ended_at: now,
        productivity: Some(92),
        mouse_clicks: Some(64),
        keyboard_hits: Some(312),
        description: Some("Implementing Phase 1 integration verification".to_string()),
        device_id: Some("dev-uuid-1".to_string()),
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
        payload_signature: None,
    }
}

// ---- The v1 contract through the CLI --------------------------------------------------------

/// A daemon on the real machine (no graphical session in the tests) with the
/// file credential store.
fn agent() -> Agent {
    let mut agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    agent.start_daemon();
    agent
}

fn login(agent: &Agent, backend: &FakeBackend, password: &str) -> std::process::Output {
    agent.cli_stdin(
        &[
            "--json",
            "login",
            "--server",
            &backend.url,
            "--email",
            EMAIL,
            "--password-stdin",
        ],
        &format!("{password}\n"),
    )
}

fn stop(agent: &mut Agent) {
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;
}

fn file_store(agent: &Agent) -> FileCredentialStore {
    FileCredentialStore::new(agent.xdg.data_dir().join("credentials.json"))
}

fn outbox(agent: &Agent) -> Outbox {
    Outbox::open(
        agent.xdg.data_dir().join("outbox.db"),
        OutboxLimits::default(),
    )
    .unwrap()
}

fn scope() -> OutboxScope {
    OutboxScope::new(EMAIL, WS_ACME).unwrap()
}

fn block(minutes_ago: i64) -> Activity {
    let start = Utc::now() - chrono::Duration::minutes(minutes_ago);
    let mut activity = Activity::new(PROJECT_WEBSITE, start, start + chrono::Duration::minutes(5));
    activity.platform = Some(PLATFORM.into());
    activity
}

/// A v2 transport from the credentials the sign-in saved.
fn transport_from_saved(agent: &Agent, server: &str) -> (HttpDeliveryTransport, [u8; 32]) {
    let creds = file_store(agent)
        .load_account(EMAIL)
        .unwrap()
        .expect("the sign-in was saved");
    assert!(
        creds.password.is_none(),
        "the file store never holds the password"
    );
    let key: [u8; 32] = creds
        .device_signing_key
        .clone()
        .expect("a device signing key")
        .try_into()
        .expect("32 bytes");
    let transport = HttpDeliveryTransport::new_shared(TransportConfig {
        server: server.to_string(),
        account: EMAIL.to_string(),
        password: None,
        tokens: TokenCell::with_refresh_token(creds.refresh_token.clone()),
        workspace_id: WS_ACME.to_string(),
        device_id: creds.device_id.clone(),
        signing_key: Some(key),
        clock: ServerClock::default(),
        allow_insecure_http: false,
    })
    .expect("a transport");
    (transport, key)
}

#[test]
fn test_cli_login_status_logout_lifecycle() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let agent = agent();

    let output = login(&agent, &backend, PASSWORD);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    let value = parse(&output);
    assert_eq!(value["result"]["account"], EMAIL);
    assert_eq!(value["result"]["workspace"], "Acme");
    for secret in [PASSWORD, "access-1", "refresh-1"] {
        assert!(
            !stdout.contains(secret) && !stderr.contains(secret),
            "{secret} printed"
        );
    }

    // The token request: password grant, the install's device id, no tenant.
    let token = &backend.requests_to("/connect/token")[0];
    let form = token.form();
    assert_eq!(form["grant_type"], "password");
    assert_eq!(form["username"], EMAIL);
    assert_eq!(form["password"], PASSWORD);
    assert!(
        uuid::Uuid::parse_str(&form["device_id"]).is_ok(),
        "{form:?}"
    );
    assert!(!token.target.contains("__tenant") && !token.body.contains("__tenant"));

    // Status.
    let snapshot = agent.wait_state(Duration::from_secs(15), |s| s["workspace"] == "Acme");
    assert_eq!(snapshot["display_name"], EMAIL);
    let text = snapshot.to_string();
    assert!(!text.contains(PASSWORD) && !text.contains("access-1") && !text.contains("refresh-1"));

    // The saved sign-in: tokens and the key, never the password.
    let raw = std::fs::read_to_string(agent.xdg.data_dir().join("credentials.json")).unwrap();
    assert!(!raw.contains(PASSWORD));
    let creds = file_store(&agent).load_account(EMAIL).unwrap().unwrap();
    assert!(creds.refresh_token.is_some() && creds.device_signing_key.is_some());

    // Logout forgets it; status fails closed for the account.
    let (code, value) = agent.json(&["logout"]);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(agent.snapshot()["status"], "signed_out");
    assert!(file_store(&agent).load_account(EMAIL).unwrap().is_none());
    let (code, value) = agent.json(&["whoami"]);
    assert_eq!(code, Some(4), "{value}");
}

#[test]
fn test_cli_status_fails_closed_and_never_creates_files() {
    let xdg = support::xdg::TestXdg::builder().create_dirs(false).build();
    let agent = Agent::with_xdg(xdg);
    let (code, value) = agent.json(&["status"]);
    assert_eq!(code, Some(3), "{value}");
    assert_eq!(value["ok"], false);
    assert_eq!(value["result"]["account"], Value::Null);
    assert!(!agent.xdg.home.exists(), "status created HOME");
}

#[test]
fn test_cli_status_with_db_reports_queue_depth() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = agent();
    assert_eq!(login(&agent, &backend, PASSWORD).status.code(), Some(0));
    stop(&mut agent);

    let (_, key) = transport_from_saved(&agent, &backend.url);
    let store = outbox(&agent);
    for minutes in [30, 20] {
        let pending = PendingActivity::from_activity(&block(minutes), Vec::new()).unwrap();
        store.enqueue_activity(&scope(), &pending, &key).unwrap();
    }
    drop(store);

    let (code, value) = agent.json(&["status"]);
    assert_eq!(code, Some(3), "the agent is down");
    assert_eq!(value["result"]["queue"]["pending"], 2, "{value}");
    let (code, value) = agent.json(&["queue", "status"]);
    assert_eq!(code, Some(0));
    assert_eq!(value["result"]["pending"], 2, "{value}");
    assert_eq!(value["result"]["source"], "local");
}

#[test]
fn test_end_to_end_signed_block_delivery_and_ack_deletion() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = agent();
    assert_eq!(login(&agent, &backend, PASSWORD).status.code(), Some(0));
    stop(&mut agent);

    let (mut transport, key) = transport_from_saved(&agent, &backend.url);
    let store = outbox(&agent);
    let activity = block(15);
    let pending = PendingActivity::from_activity(&activity, Vec::new()).unwrap();
    store.enqueue_activity(&scope(), &pending, &key).unwrap();
    let queued = store.peek_ready(&scope(), 10).unwrap();
    assert_eq!(queued.len(), 1);
    assert!(
        verify_v2_entry(&key, &queued[0]),
        "the row is signed with the saved key"
    );

    let report = SyncPassCoordinator::new(&store, SyncPassConfig::default())
        .run_pass(&scope(), &mut transport)
        .unwrap();
    assert_eq!(
        report.persisted_acknowledged,
        vec![activity.client_activity_id.clone()]
    );
    assert!(report.transient_failures.is_empty());

    // The saved refresh token renewed the sign-in, with the device id.
    let refresh = backend
        .requests_to("/connect/token")
        .into_iter()
        .find(|r| r.form().get("grant_type").map(String::as_str) == Some("refresh_token"))
        .expect("a refresh grant");
    assert!(refresh.form().contains_key("device_id"));
    // The upload: the row's workspace, the stored signature, null counts.
    let upload = &backend.requests_to("/api/app/activity")[0];
    assert_eq!(upload.header("x-workspace-id"), Some(WS_ACME));
    let body = upload.json();
    assert_eq!(
        body["payloadSignature"].as_str(),
        queued[0].payload_signature.as_deref()
    );
    assert!(body["keyboardHits"].is_null() && body["mouseClicks"].is_null());

    // Acknowledged: deleted.
    assert!(store.peek_ready(&scope(), 10).unwrap().is_empty());
    drop(store);
    let (_, value) = agent.json(&["queue", "status"]);
    assert_eq!(value["result"]["pending"], 0, "{value}");
}

#[test]
fn test_end_to_end_mid_upload_server_error_retains_row_and_increments_retry() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = agent();
    assert_eq!(login(&agent, &backend, PASSWORD).status.code(), Some(0));
    stop(&mut agent);
    backend.with(|s| s.activity_mode = ActivityMode::Unavailable);

    let (mut transport, key) = transport_from_saved(&agent, &backend.url);
    let store = outbox(&agent);
    let activity = block(15);
    store
        .enqueue_activity(
            &scope(),
            &PendingActivity::from_activity(&activity, Vec::new()).unwrap(),
            &key,
        )
        .unwrap();
    // Another account's row must not be touched.
    let sentinel = OutboxScope::new("other-user@dosi.test", WS_ACME).unwrap();
    store
        .enqueue_activity(
            &sentinel,
            &PendingActivity::from_activity(&block(40), Vec::new()).unwrap(),
            &key,
        )
        .unwrap();

    let report = SyncPassCoordinator::new(&store, SyncPassConfig::default())
        .run_pass(&scope(), &mut transport)
        .unwrap();
    assert!(report.persisted_acknowledged.is_empty());
    assert_eq!(report.transient_failures.len(), 1);
    assert_eq!(
        report.transient_failures[0].client_activity_id,
        activity.client_activity_id
    );

    let kept = store.peek_ready(&scope(), 10).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].retry_count, 1);
    assert!(kept[0].last_error.is_some());
    let other = store.peek_ready(&sentinel, 10).unwrap();
    assert_eq!((other.len(), other[0].retry_count), (1, 0));
    assert!(other[0].last_error.is_none());
    drop(store);
    let (_, value) = agent.json(&["status"]);
    assert_eq!(value["result"]["queue"]["pending"], 1, "{value}");
}

#[test]
fn test_end_to_end_mid_upload_socket_interruption_retains_row_and_increments_retry() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = agent();
    assert_eq!(login(&agent, &backend, PASSWORD).status.code(), Some(0));
    stop(&mut agent);

    let (server_url, rx, server) = interrupting_server();
    let (mut transport, key) = transport_from_saved(&agent, &server_url);
    let store = outbox(&agent);
    let activity = block(15);
    store
        .enqueue_activity(
            &scope(),
            &PendingActivity::from_activity(&activity, Vec::new()).unwrap(),
            &key,
        )
        .unwrap();

    let report = SyncPassCoordinator::new(&store, SyncPassConfig::default())
        .run_pass(&scope(), &mut transport)
        .unwrap();
    let token = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(token.contains("grant_type=refresh_token"), "{token}");
    let upload = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(upload.contains("POST /api/app/activity"));
    server.join().unwrap();

    assert!(report.persisted_acknowledged.is_empty());
    assert_eq!(report.transient_failures.len(), 1);
    let kept = store.peek_ready(&scope(), 10).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].retry_count, 1);
    assert_eq!(kept[0].client_activity_id, activity.client_activity_id);
}

#[test]
fn test_failed_login_fails_closed_and_cannot_be_counted_as_delivery_evidence() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = agent();
    let output = login(&agent, &backend, "wrong-password");
    assert_eq!(output.status.code(), Some(4));
    let value = parse(&output);
    assert_eq!(value["error"]["code"], "sign_in_failed");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!all.contains("wrong-password"));
    assert_eq!(agent.snapshot()["status"], "signed_out");
    stop(&mut agent);
    // Nothing was saved: no transport can be built from it.
    assert!(file_store(&agent).load_account(EMAIL).unwrap().is_none());
    assert!(backend.requests_to("/api/app/activity").is_empty());
}

// ---- Transport scope binding (library level) ------------------------------------------------

#[test]
fn test_transport_scope_binding_account_mismatch_same_workspace() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_target = OutboxScope::new("account-b@dosi.test", "workspace-shared").unwrap();
    let envelope = sample_envelope("act-mismatch-account");
    outbox.enqueue(&scope_target, &envelope).unwrap();

    let (url, _rx, counter, _shutdown) = counting_server_loop(vec![token_ok()]);

    let store = InMemoryCredentialStore::new();
    let mut creds = Credentials::new("account-a@dosi.test", Some("pw-a".into()));
    creds.workspace_id = Some("workspace-shared".into());
    creds.server_url = Some(url);
    store.save(&creds).unwrap();

    let mut transport =
        HttpDeliveryTransport::from_store(&store, "account-a@dosi.test", Some("workspace-shared"))
            .expect("Transport construction should succeed for valid credentials");

    assert_eq!(transport.bound_account_id(), "account-a@dosi.test");
    assert_eq!(transport.bound_workspace_id(), "workspace-shared");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());
    let report = coordinator.run_pass(&scope_target, &mut transport).unwrap();

    // Invariant: DefinitiveRejection on scope mismatch
    match report.stop_reason {
        Some(SyncStopReason::DefinitiveRejection {
            ref client_activity_id,
            ref reason,
        }) => {
            assert_eq!(client_activity_id, "act-mismatch-account");
            assert!(reason.contains("Scope mismatch"));
            assert!(reason.contains("account-a@dosi.test"));
            assert!(reason.contains("account-b@dosi.test"));
            // Invariant: no secrets leaked
            assert!(!reason.contains("pw-a"));
        }
        other => panic!("Expected DefinitiveRejection, got {other:?}"),
    }

    assert_eq!(report.transport_accepted.len(), 0);
    assert_eq!(report.persisted_acknowledged.len(), 0);

    // Invariant: Zero network bytes/requests reach the transport HTTP backend
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "No HTTP requests must reach the network"
    );

    // Invariant: Row preserved and retry state intact
    let pending = outbox.peek_pending(&scope_target, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-mismatch-account");
    assert_eq!(pending[0].retry_count, 0);
    assert!(pending[0].last_error.is_none());
}

#[test]
fn test_transport_scope_binding_workspace_mismatch_same_account() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_target = OutboxScope::new("account-shared@dosi.test", "workspace-b").unwrap();
    let envelope = sample_envelope("act-mismatch-workspace");
    outbox.enqueue(&scope_target, &envelope).unwrap();

    let (url, _rx, counter, _shutdown) = counting_server_loop(vec![token_ok()]);

    let store = InMemoryCredentialStore::new();
    let mut creds = Credentials::new("account-shared@dosi.test", Some("pw-shared".into()));
    creds.workspace_id = Some("workspace-a".into());
    creds.server_url = Some(url);
    store.save(&creds).unwrap();

    let mut transport =
        HttpDeliveryTransport::from_store(&store, "account-shared@dosi.test", Some("workspace-a"))
            .expect("Transport construction should succeed for valid credentials");

    assert_eq!(transport.bound_account_id(), "account-shared@dosi.test");
    assert_eq!(transport.bound_workspace_id(), "workspace-a");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());
    let report = coordinator.run_pass(&scope_target, &mut transport).unwrap();

    match report.stop_reason {
        Some(SyncStopReason::DefinitiveRejection {
            ref client_activity_id,
            ref reason,
        }) => {
            assert_eq!(client_activity_id, "act-mismatch-workspace");
            assert!(reason.contains("Scope mismatch"));
            assert!(reason.contains("workspace-a"));
            assert!(reason.contains("workspace-b"));
            assert!(!reason.contains("pw-shared"));
        }
        other => panic!("Expected DefinitiveRejection, got {other:?}"),
    }

    assert_eq!(report.transport_accepted.len(), 0);
    assert_eq!(report.persisted_acknowledged.len(), 0);

    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "No HTTP requests must reach the network"
    );

    let pending = outbox.peek_pending(&scope_target, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].retry_count, 0);
    assert!(pending[0].last_error.is_none());
}

#[test]
fn test_transport_scope_binding_both_mismatch() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_target = OutboxScope::new("account-b@dosi.test", "workspace-b").unwrap();
    let envelope = sample_envelope("act-mismatch-both");
    outbox.enqueue(&scope_target, &envelope).unwrap();

    let (url, _rx, counter, _shutdown) = counting_server_loop(vec![token_ok()]);

    let store = InMemoryCredentialStore::new();
    let mut creds = Credentials::new("account-a@dosi.test", Some("pw-a".into()));
    creds.workspace_id = Some("workspace-a".into());
    creds.server_url = Some(url);
    store.save(&creds).unwrap();

    let mut transport =
        HttpDeliveryTransport::from_store(&store, "account-a@dosi.test", Some("workspace-a"))
            .expect("Transport construction should succeed for valid credentials");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());
    let report = coordinator.run_pass(&scope_target, &mut transport).unwrap();

    assert!(matches!(
        report.stop_reason,
        Some(SyncStopReason::DefinitiveRejection { .. })
    ));
    assert_eq!(report.persisted_acknowledged.len(), 0);

    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    let pending = outbox.peek_pending(&scope_target, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].retry_count, 0);
    assert!(pending[0].last_error.is_none());
}

#[test]
fn test_transport_scope_binding_absent_and_empty_workspace_fail_closed() {
    let store = InMemoryCredentialStore::new();

    // ApiClient without workspace
    let client_no_ws = ApiClient::new("http://127.0.0.1:9", "acc@dosi.test".into(), "pw".into());
    let err_client_no_ws = HttpDeliveryTransport::new(client_no_ws);
    assert!(err_client_no_ws.is_err());
    assert!(err_client_no_ws
        .unwrap_err()
        .contains("requires a bound workspace_id"));

    // ApiClient with empty workspace
    let client_empty_ws = ApiClient::new("http://127.0.0.1:9", "acc@dosi.test".into(), "pw".into())
        .with_workspace(Some("".into()));
    assert!(HttpDeliveryTransport::new(client_empty_ws).is_err());

    // ApiClient with whitespace workspace
    let client_ws_spaces =
        ApiClient::new("http://127.0.0.1:9", "acc@dosi.test".into(), "pw".into())
            .with_workspace(Some("   ".into()));
    assert!(HttpDeliveryTransport::new(client_ws_spaces).is_err());

    // Credentials without workspace
    let mut creds_no_ws = Credentials::new("acc@dosi.test", Some("pw".into()));
    creds_no_ws.workspace_id = None;
    let err_creds_no_ws = HttpDeliveryTransport::from_credentials(&creds_no_ws);
    assert!(err_creds_no_ws.is_err());
    assert!(err_creds_no_ws.unwrap_err().contains("bound workspace_id"));

    // Credentials with whitespace workspace
    let mut creds_empty_ws = Credentials::new("acc@dosi.test", Some("pw".into()));
    creds_empty_ws.workspace_id = Some("  \t  ".into());
    let err_creds_empty_ws = HttpDeliveryTransport::from_credentials(&creds_empty_ws);
    assert!(err_creds_empty_ws.is_err());
    assert!(err_creds_empty_ws
        .unwrap_err()
        .contains("non-empty workspace_id"));

    // from_store with None
    let err_store_none = HttpDeliveryTransport::from_store(&store, "acc@dosi.test", None);
    assert!(err_store_none.is_err());
    assert!(err_store_none
        .unwrap_err()
        .contains("explicit bound workspace_id"));

    // from_store with empty / whitespace
    let err_store_empty = HttpDeliveryTransport::from_store(&store, "acc@dosi.test", Some(""));
    assert!(err_store_empty.is_err());
    assert!(err_store_empty
        .unwrap_err()
        .contains("non-empty workspace_id"));

    let err_store_whitespace =
        HttpDeliveryTransport::from_store(&store, "acc@dosi.test", Some("   "));
    assert!(err_store_whitespace.is_err());
    assert!(err_store_whitespace
        .unwrap_err()
        .contains("non-empty workspace_id"));
}

#[test]
fn test_transport_scope_binding_same_scope_positive_delivery_and_exact_deletion() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope = OutboxScope::new("account-pos@dosi.test", "workspace-pos").unwrap();
    let envelope = sample_envelope("act-pos-match-1");
    outbox.enqueue(&scope, &envelope).unwrap();

    let (url, rx, counter, _shutdown) = counting_server_loop(vec![
        token_ok(),
        http_resp("200 OK", &[], r#"{"clientActivityId":"act-pos-match-1"}"#),
    ]);

    let store = InMemoryCredentialStore::new();
    let mut creds = Credentials::new("account-pos@dosi.test", Some("pw-pos".into()));
    creds.workspace_id = Some("workspace-pos".into());
    creds.server_url = Some(url);
    store.save(&creds).unwrap();

    let mut transport =
        HttpDeliveryTransport::from_store(&store, "account-pos@dosi.test", Some("workspace-pos"))
            .expect("Transport construction should succeed");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());
    let report = coordinator.run_pass(&scope, &mut transport).unwrap();

    assert_eq!(
        report.transport_accepted,
        vec!["act-pos-match-1".to_string()]
    );
    assert_eq!(
        report.persisted_acknowledged,
        vec!["act-pos-match-1".to_string()]
    );
    assert_eq!(report.transient_failures.len(), 0);
    assert_eq!(report.definitive_rejections.len(), 0);

    // Invariant: Exactly deleted from outbox
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 0);

    // Invariant: Activity request was received with expected workspace header
    let _token_req = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("Token request received");
    let upload_req = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("Upload request received");
    assert!(upload_req.contains("POST /api/app/activity"));
    let req_lower = upload_req.to_lowercase();
    assert!(req_lower.contains("x-workspace-id: workspace-pos"));
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[test]
fn test_transport_scope_binding_scoped_queue_coexistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_1 = OutboxScope::new("user-1@dosi.test", "ws-tenant-1").unwrap();
    let scope_2 = OutboxScope::new("user-2@dosi.test", "ws-tenant-2").unwrap();

    let env_1 = sample_envelope("act-coexist-1");
    let env_2 = sample_envelope("act-coexist-2");
    outbox.enqueue(&scope_1, &env_1).unwrap();
    outbox.enqueue(&scope_2, &env_2).unwrap();

    let (url, _rx, _counter, _shutdown) = counting_server_loop(vec![
        token_ok(),
        http_resp("200 OK", &[], r#"{"clientActivityId":"act-coexist-1"}"#),
        token_ok(),
        http_resp("200 OK", &[], r#"{"clientActivityId":"act-coexist-2"}"#),
    ]);

    let store = InMemoryCredentialStore::new();
    let mut creds_1 = Credentials::new("user-1@dosi.test", Some("pw-1".into()));
    creds_1.workspace_id = Some("ws-tenant-1".into());
    creds_1.server_url = Some(url.clone());
    store.save(&creds_1).unwrap();

    let mut creds_2 = Credentials::new("user-2@dosi.test", Some("pw-2".into()));
    creds_2.workspace_id = Some("ws-tenant-2".into());
    creds_2.server_url = Some(url);
    store.save(&creds_2).unwrap();

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

    // Execute pass 1 with transport 1
    let mut transport_1 =
        HttpDeliveryTransport::from_store(&store, "user-1@dosi.test", Some("ws-tenant-1")).unwrap();
    let report_1 = coordinator.run_pass(&scope_1, &mut transport_1).unwrap();
    assert_eq!(
        report_1.persisted_acknowledged,
        vec!["act-coexist-1".to_string()]
    );

    // Verify scope_1 queue drained and scope_2 completely untouched
    assert_eq!(outbox.count(Some(&scope_1)).unwrap(), 0);
    assert_eq!(outbox.count(Some(&scope_2)).unwrap(), 1);

    let pending_2 = outbox.peek_pending(&scope_2, 10).unwrap();
    assert_eq!(pending_2.len(), 1);
    assert_eq!(pending_2[0].client_activity_id, "act-coexist-2");
    assert_eq!(pending_2[0].retry_count, 0);
    assert!(pending_2[0].last_error.is_none());

    // Execute pass 2 with transport 2
    let mut transport_2 =
        HttpDeliveryTransport::from_store(&store, "user-2@dosi.test", Some("ws-tenant-2")).unwrap();
    let report_2 = coordinator.run_pass(&scope_2, &mut transport_2).unwrap();
    assert_eq!(
        report_2.persisted_acknowledged,
        vec!["act-coexist-2".to_string()]
    );

    // Both queues now drained
    assert_eq!(outbox.count(Some(&scope_1)).unwrap(), 0);
    assert_eq!(outbox.count(Some(&scope_2)).unwrap(), 0);
}

#[test]
fn test_transport_scope_binding_native_failbefore_reproduction() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    // Foreign scope queued entry
    let scope_foreign = OutboxScope::new("account-b@dosi.test", "workspace-b").unwrap();
    let env_foreign = sample_envelope("act-native-failbefore-check");
    outbox.enqueue(&scope_foreign, &env_foreign).unwrap();

    // Server tracking activity requests
    let activity_requests = Arc::new(AtomicUsize::new(0));
    let activity_requests_clone = activity_requests.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let _ = listener.set_nonblocking(true);
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        while shutdown_rx.try_recv().is_err() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let req = read_request(&mut stream);
                    if req.contains("POST /api/app/activity") {
                        activity_requests_clone.fetch_add(1, Ordering::SeqCst);
                    }
                    let resp = if req.contains("POST /connect/token") {
                        token_ok()
                    } else {
                        http_resp(
                            "200 OK",
                            &[],
                            r#"{"clientActivityId":"act-native-failbefore-check"}"#,
                        )
                    };
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    // Transport initialized for account-a/workspace-a
    let store = InMemoryCredentialStore::new();
    let mut creds_a = Credentials::new("account-a@dosi.test", Some("pw-a".into()));
    creds_a.workspace_id = Some("workspace-a".into());
    creds_a.server_url = Some(url);
    store.save(&creds_a).unwrap();

    let mut transport_a =
        HttpDeliveryTransport::from_store(&store, "account-a@dosi.test", Some("workspace-a"))
            .unwrap();

    // Coordinator runs pass with foreign scope
    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());
    let report = coordinator
        .run_pass(&scope_foreign, &mut transport_a)
        .unwrap();

    // Evaluate reproduction assertions
    let cross_scope_activity_requests = activity_requests.load(Ordering::SeqCst);
    let foreign_rows_remaining = outbox.count(Some(&scope_foreign)).unwrap();
    let persisted_acknowledged = report.persisted_acknowledged.len();

    // Native fail-before observed:
    // cross_scope_activity_requests=1
    // foreign_rows_remaining=0
    // persisted_acknowledged=1
    // PROBE_EXIT=101
    //
    // Fixed transport identity binding asserts:
    assert_eq!(
        cross_scope_activity_requests, 0,
        "foreign-scope row must never reach HTTP transport: left{cross_scope_activity_requests} right0"
    );
    assert_eq!(
        foreign_rows_remaining, 1,
        "foreign-scope row must remain in outbox without deletion"
    );
    assert_eq!(
        persisted_acknowledged, 0,
        "foreign-scope row must never be acknowledged"
    );
    assert!(
        matches!(
            report.stop_reason,
            Some(SyncStopReason::DefinitiveRejection { .. })
        ),
        "pass must halt on definitive rejection without advancing or deleting foreign rows"
    );

    let _ = shutdown_tx.send(());
}
