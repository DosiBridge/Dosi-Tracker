//! End-to-end integration tests for Phase 1 identity, transport, and signing.
//!
//! Enforces:
//! - Complete CLI login -> status -> queue -> signed delivery -> ack delete lifecycle.
//! - Mid-upload outage row retention and attempt counter incrementation (HTTP 502 response and TCP socket interruption).
//! - Subprocess execution assertions and fail-closed security invariants: failed logins never count as delivery evidence.
//! - Reusing credentials persisted by CLI login for production upload transport (`from_store`).
//! - Read-only inspection invariants: status and doctor never create files or directories.
//! - Deterministic execution over in-process loopback fixtures without external network access.
//! - Complete redaction: zero secrets in CLI stdout, stderr, or JSON reports.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tempfile::tempdir;

use dosi_tracker_linux::api::{ApiClient, HttpDeliveryTransport};
use dosi_tracker_linux::credentials::{
    CredentialStore, Credentials, FileCredentialStore, InMemoryCredentialStore,
};
use dosi_tracker_linux::outbox::{ActivityEnvelope, Outbox, OutboxLimits, OutboxScope};
use dosi_tracker_linux::signing::verify_activity_envelope;
use dosi_tracker_linux::sync::{SyncPassConfig, SyncPassCoordinator, SyncStopReason};

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

fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
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

fn interrupting_server() -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        // Step 1: Token request for login
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let req = read_request(&mut stream);
            let _ = tx.send(req);
            let resp = token_ok();
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
        // Step 2: Activity upload request -> observe request headers/body, then abruptly drop/shutdown
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
            drop(stream);
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

#[test]
fn test_cli_login_status_logout_lifecycle() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");

    let (url, rx) = canned_server(vec![token_ok()]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. Login via CLI
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "tester@dosi.test",
            "--password",
            "super-secret-password",
            "--workspace-id",
            "ws-123",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login failed with status code {:?}",
        login_out.status.code()
    );
    let login_stdout = String::from_utf8_lossy(&login_out.stdout);
    assert!(login_stdout.contains("tester@dosi.test"));
    assert!(login_stdout.contains("ws-123"));
    assert!(!login_stdout.contains("super-secret-password"));
    assert!(!login_stdout.contains("mock-token-xyz"));

    // Verify token request parameters sent to mock server
    let token_req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Token request received");
    assert!(token_req.contains("grant_type=password"));
    assert!(token_req.contains("username=tester%40dosi.test"));
    assert!(token_req.contains("password=super-secret-password"));

    // 2. Status inspection via CLI (JSON)
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "tester@dosi.test",
            "--workspace-id",
            "ws-123",
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        status_out.status.success(),
        "Status failed with status code {:?}",
        status_out.status.code()
    );
    let status_stdout = String::from_utf8_lossy(&status_out.stdout);
    let json: serde_json::Value = serde_json::from_str(&status_stdout).expect("Valid JSON");
    assert_eq!(json["account_id"], "tester@dosi.test");
    assert_eq!(json["workspace_id"], "ws-123");
    assert_eq!(json["token_valid"], true);
    assert_eq!(json["has_signing_key"], true);
    assert!(!status_stdout.contains("super-secret-password"));
    assert!(!status_stdout.contains("mock-token-xyz"));

    // 3. Logout via CLI
    let logout_out = Command::new(exe)
        .args([
            "logout",
            "--account-id",
            "tester@dosi.test",
            "--workspace-id",
            "ws-123",
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute logout command");

    assert!(
        logout_out.status.success(),
        "Logout failed with status code {:?}",
        logout_out.status.code()
    );

    // 4. Status after logout must fail closed
    let status_after = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "tester@dosi.test",
            "--workspace-id",
            "ws-123",
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(!status_after.status.success());
}

#[test]
fn test_cli_status_fails_closed_and_never_creates_files() {
    let dir = tempdir().unwrap();
    let nonexistent_file = dir.path().join("subdir").join("creds.json");

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let output = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "missing@dosi.test",
            "--workspace-id",
            "ws-missing",
            "--credentials-file",
            nonexistent_file.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(!output.status.success());
    assert!(!nonexistent_file.exists());
    assert!(!dir.path().join("subdir").exists());
}

#[test]
fn test_cli_status_with_db_reports_queue_depth() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let cred_path = dir.path().join("credentials.json");

    let (url, rx) = canned_server(vec![token_ok()]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // Login via CLI and assert exit status
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "queue-user@dosi.test",
            "--password",
            "pass",
            "--workspace-id",
            "ws-q",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login subprocess failed with exit code {:?}",
        login_out.status.code()
    );
    let _ = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Token request received");

    // Enqueue 2 entries in outbox
    let scope = OutboxScope::new("queue-user@dosi.test", "ws-q").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    outbox.enqueue(&scope, &sample_envelope("act-1")).unwrap();
    outbox.enqueue(&scope, &sample_envelope("act-2")).unwrap();

    // Status inspection with --db
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "queue-user@dosi.test",
            "--workspace-id",
            "ws-q",
            "--db",
            db_path.to_str().unwrap(),
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        status_out.status.success(),
        "Status command failed with exit code {:?}",
        status_out.status.code()
    );
    let stdout = String::from_utf8_lossy(&status_out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["queue_depth"], 2);
}

#[test]
fn test_end_to_end_signed_block_delivery_and_ack_deletion() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let cred_path = dir.path().join("credentials.json");

    let (server_url, rx) = canned_server(vec![
        token_ok(),
        http_resp("200 OK", &[], r#"{"clientActivityId":"act-signed-slice"}"#),
    ]);
    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. Login via CLI and assert successful status
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "signer@dosi.test",
            "--password",
            "pw",
            "--workspace-id",
            "ws-sign",
            "--server",
            &server_url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login subprocess failed with exit code {:?}",
        login_out.status.code()
    );
    let login_req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Login request received");
    assert!(login_req.contains("grant_type=password"));

    // 2. Enqueue activity in outbox
    let scope = OutboxScope::new("signer@dosi.test", "ws-sign").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let envelope = sample_envelope("act-signed-slice");
    outbox.enqueue(&scope, &envelope).unwrap();

    // 3. Construct transport directly from persisted credentials in store (reusing session)
    let store = FileCredentialStore::new(cred_path.clone());
    let mut transport =
        HttpDeliveryTransport::from_store(&store, "signer@dosi.test", Some("ws-sign"))
            .expect("Failed to construct HttpDeliveryTransport from persisted store");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

    // 4. Execute sync pass using persisted credentials
    let report = coordinator.run_pass(&scope, &mut transport).unwrap();
    assert_eq!(
        report.persisted_acknowledged,
        vec!["act-signed-slice".to_string()]
    );
    assert_eq!(report.transient_failures.len(), 0);

    // 5. Verify payload signature received by canned backend
    let upload_req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Upload request received");
    assert!(upload_req.contains("payloadSignature"));

    // Extract signature and verify with signing key from store
    let creds = store
        .load("signer@dosi.test", Some("ws-sign"))
        .unwrap()
        .expect("Credentials persisted");
    let key_bytes: [u8; 32] = creds
        .device_signing_key
        .expect("Signing key present")
        .try_into()
        .expect("32 bytes");

    let body_start = upload_req.find("\r\n\r\n").unwrap() + 4;
    let body_json = &upload_req[body_start..];
    let parsed: serde_json::Value = serde_json::from_str(body_json).unwrap();
    let sig = parsed["payloadSignature"].as_str().unwrap();

    let mut env_check = envelope.clone();
    env_check.payload_signature = Some(sig.to_string());
    assert!(
        verify_activity_envelope(&key_bytes, &env_check),
        "Uploaded envelope signature must be mathematically valid"
    );

    // 6. Verify outbox queue: entry deleted after positive ack
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 0);

    // 7. Verify status reports queue depth = 0
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "signer@dosi.test",
            "--workspace-id",
            "ws-sign",
            "--db",
            db_path.to_str().unwrap(),
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        status_out.status.success(),
        "Status command failed with exit code {:?}",
        status_out.status.code()
    );
    let status_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&status_out.stdout)).unwrap();
    assert_eq!(status_json["queue_depth"], 0);
}

#[test]
fn test_end_to_end_mid_upload_server_502_error_retains_row_and_increments_retry() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let cred_path = dir.path().join("credentials.json");

    let (server_url, rx) = canned_server(vec![
        token_ok(),
        http_resp("502 Bad Gateway", &[], r#"{"error":"gateway timeout"}"#),
    ]);
    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // Login via CLI and assert successful status
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "outage-user@dosi.test",
            "--password",
            "pw",
            "--workspace-id",
            "ws-outage",
            "--server",
            &server_url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login subprocess failed with exit code {:?}",
        login_out.status.code()
    );
    let _ = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Login request received");

    // Enqueue activity in Target Scope
    let scope = OutboxScope::new("outage-user@dosi.test", "ws-outage").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let envelope = sample_envelope("act-mid-outage");
    outbox.enqueue(&scope, &envelope).unwrap();

    // Enqueue sentinel activity in Unrelated Scope B to verify no cross-scope mutation
    let sentinel_scope = OutboxScope::new("other-user@dosi.test", "ws-sentinel").unwrap();
    let sentinel_env = sample_envelope("act-sentinel-502");
    outbox.enqueue(&sentinel_scope, &sentinel_env).unwrap();

    // Construct transport directly from persisted store (reusing session)
    let store = FileCredentialStore::new(cred_path.clone());
    let mut transport =
        HttpDeliveryTransport::from_store(&store, "outage-user@dosi.test", Some("ws-outage"))
            .expect("Failed to construct transport from store");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

    // Execute sync pass: upload fails transiently on 502 Bad Gateway
    let report = coordinator.run_pass(&scope, &mut transport).unwrap();
    assert_eq!(report.persisted_acknowledged.len(), 0);
    assert_eq!(report.transient_failures.len(), 1);
    assert_eq!(
        report.transient_failures[0].client_activity_id,
        "act-mid-outage"
    );

    // Verify target row is RETAINED with retry_count incremented to 1
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-mid-outage");
    assert_eq!(pending[0].retry_count, 1);
    assert!(pending[0].last_error.is_some());
    assert_eq!(pending[0].envelope.description, envelope.description);

    // Verify sentinel row in unrelated scope is completely untouched
    let sentinel_pending = outbox.peek_pending(&sentinel_scope, 10).unwrap();
    assert_eq!(sentinel_pending.len(), 1);
    assert_eq!(sentinel_pending[0].client_activity_id, "act-sentinel-502");
    assert_eq!(sentinel_pending[0].retry_count, 0);
    assert_eq!(sentinel_pending[0].last_error, None);

    // Verify status reports queue depth = 1
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "outage-user@dosi.test",
            "--workspace-id",
            "ws-outage",
            "--db",
            db_path.to_str().unwrap(),
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        status_out.status.success(),
        "Status command failed with exit code {:?}",
        status_out.status.code()
    );
    let status_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&status_out.stdout)).unwrap();
    assert_eq!(status_json["queue_depth"], 1);
}

#[test]
fn test_end_to_end_mid_upload_socket_interruption_retains_row_and_increments_retry() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let cred_path = dir.path().join("credentials.json");

    let (server_url, rx, server_handle) = interrupting_server();
    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. CLI Login against server
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "interrupt-user@dosi.test",
            "--password",
            "pw",
            "--workspace-id",
            "ws-interrupt",
            "--server",
            &server_url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login subprocess failed with exit code {:?}",
        login_out.status.code()
    );
    let login_req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Login request received");
    assert!(login_req.contains("grant_type=password"));

    // 2. Enqueue activity in target scope
    let scope = OutboxScope::new("interrupt-user@dosi.test", "ws-interrupt").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let envelope = sample_envelope("act-interrupted-socket");
    outbox.enqueue(&scope, &envelope).unwrap();

    // 3. Enqueue sentinel activity in unrelated scope to verify cross-scope isolation
    let sentinel_scope = OutboxScope::new("sentinel-user@dosi.test", "ws-sentinel").unwrap();
    let sentinel_env = sample_envelope("act-sentinel-intact");
    outbox.enqueue(&sentinel_scope, &sentinel_env).unwrap();

    // 4. Construct transport directly from store (reusing session)
    let store = FileCredentialStore::new(cred_path.clone());
    let mut transport =
        HttpDeliveryTransport::from_store(&store, "interrupt-user@dosi.test", Some("ws-interrupt"))
            .expect("Failed to construct transport from store");

    let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

    // 5. Execute sync pass: upload is interrupted mid-upload without valid HTTP response
    let report = coordinator.run_pass(&scope, &mut transport).unwrap();

    // Wait for server thread with bound
    let upload_req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Upload request observed by server before interruption");
    assert!(upload_req.contains("POST /api/app/activity"));
    server_handle
        .join()
        .expect("Server thread finished cleanly");

    // 6. Assertions on delivery outcome:
    // No acknowledged deletion
    assert_eq!(report.persisted_acknowledged.len(), 0);
    // Transient failure reported
    assert_eq!(report.transient_failures.len(), 1);
    assert_eq!(
        report.transient_failures[0].client_activity_id,
        "act-interrupted-socket"
    );

    // 7. Verify outbox row in target scope is RETAINED with retry_count incremented to 1
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-interrupted-socket");
    assert_eq!(pending[0].retry_count, 1);
    assert!(pending[0].last_error.is_some());
    assert_eq!(pending[0].envelope.description, envelope.description);
    assert_eq!(pending[0].envelope.started_at, envelope.started_at);

    // 8. Verify sentinel row in unrelated scope is completely untouched
    let sentinel_pending = outbox.peek_pending(&sentinel_scope, 10).unwrap();
    assert_eq!(sentinel_pending.len(), 1);
    assert_eq!(
        sentinel_pending[0].client_activity_id,
        "act-sentinel-intact"
    );
    assert_eq!(sentinel_pending[0].retry_count, 0);
    assert_eq!(sentinel_pending[0].last_error, None);

    // 9. Verify CLI status on target scope reports queue depth = 1
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "interrupt-user@dosi.test",
            "--workspace-id",
            "ws-interrupt",
            "--db",
            db_path.to_str().unwrap(),
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        status_out.status.success(),
        "Status command failed with exit code {:?}",
        status_out.status.code()
    );
    let status_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&status_out.stdout)).unwrap();
    assert_eq!(status_json["queue_depth"], 1);

    // 10. Verify CLI status on sentinel scope (has no credentials, fails closed)
    let status_sentinel = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "sentinel-user@dosi.test",
            "--workspace-id",
            "ws-sentinel",
            "--db",
            db_path.to_str().unwrap(),
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute status command on sentinel");

    assert!(!status_sentinel.status.success());
}

#[test]
fn test_failed_login_fails_closed_and_cannot_be_counted_as_delivery_evidence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let cred_path = dir.path().join("credentials.json");

    let (url, rx) = canned_server(vec![http_resp(
        "400 Bad Request",
        &[],
        r#"{"error":"invalid_grant","error_description":"Incorrect username or password."}"#,
    )]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. Attempt login with wrong password
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "rejected-user@dosi.test",
            "--password",
            "wrong-password",
            "--workspace-id",
            "ws-rejected",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    // Assert login fails with non-zero exit code
    assert!(
        !login_out.status.success(),
        "Expected login to fail for invalid credentials"
    );
    assert_eq!(login_out.status.code(), Some(1));

    // Assert safe diagnostics: zero credentials leaked
    let stderr = String::from_utf8_lossy(&login_out.stderr);
    assert!(stderr.contains("Authentication failed"));
    assert!(!stderr.contains("wrong-password"));

    let _ = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Token request observed");

    // 2. Assert no credentials stored for this scope
    let store = FileCredentialStore::new(cred_path.clone());
    let loaded = store
        .load("rejected-user@dosi.test", Some("ws-rejected"))
        .unwrap();
    assert_eq!(
        loaded, None,
        "Failed login must never persist credentials to store"
    );

    // 3. Status inspection must fail closed
    let status_out = Command::new(exe)
        .args([
            "status",
            "--account-id",
            "rejected-user@dosi.test",
            "--workspace-id",
            "ws-rejected",
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute status command");

    assert!(
        !status_out.status.success(),
        "Status must fail closed when credentials are absent"
    );

    // 4. Enqueue activity in outbox
    let scope = OutboxScope::new("rejected-user@dosi.test", "ws-rejected").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let env = sample_envelope("act-unauth");
    outbox.enqueue(&scope, &env).unwrap();

    // 5. Attempting to construct transport from store fails closed
    let transport_err =
        HttpDeliveryTransport::from_store(&store, "rejected-user@dosi.test", Some("ws-rejected"));
    assert!(
        transport_err.is_err(),
        "Transport cannot be created from non-existent credentials"
    );

    // 6. Verify outbox row remains untouched with retry_count = 0, zero deletions
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-unauth");
    assert_eq!(pending[0].retry_count, 0);
    assert_eq!(pending[0].last_error, None);
}

#[test]
fn test_cli_login_short_grant_stores_actual_expiry_upper_bound() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");

    // Loopback in-process fake token endpoint returning short expires_in: 5
    let short_token_response = http_resp(
        "200 OK",
        &[],
        r#"{"access_token":"short-live-token-5s","token_type":"Bearer","expires_in":5,"refresh_token":"short-refresh-5s"}"#,
    );
    let (url, _rx) = canned_server(vec![short_token_response]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let login_time_before = Utc::now();

    // Actual CLI login via subprocess
    let login_out = Command::new(exe)
        .args([
            "login",
            "--username",
            "short-tester@dosi.test",
            "--password",
            "test-pass",
            "--workspace-id",
            "ws-short",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute login command");

    assert!(
        login_out.status.success(),
        "Login failed with status code {:?}",
        login_out.status.code()
    );

    let login_time_after = Utc::now();

    // Inspect persisted credentials from disk
    let store = FileCredentialStore::new(cred_path);
    let creds = store
        .load("short-tester@dosi.test", Some("ws-short"))
        .expect("Store load should succeed")
        .expect("Credentials must be saved");

    assert_eq!(creds.access_token.as_deref(), Some("short-live-token-5s"));
    assert_eq!(creds.refresh_token.as_deref(), Some("short-refresh-5s"));

    let expires_at = creds
        .access_token_expires_at
        .expect("access_token_expires_at must be populated from server response");

    // Defect regression check: Must NEVER be extended to 3600 seconds!
    // The delta from login time must be bounded by the actual 5-second grant (e.g. <= 6s).
    let delta_from_before = (expires_at - login_time_before).num_seconds();
    let delta_from_after = (expires_at - login_time_after).num_seconds();

    assert!(
        delta_from_before <= 6,
        "Persisted expiry delta ({}s) exceeded actual server grant (5s); token was extended!",
        delta_from_before
    );
    assert!(
        delta_from_after >= 3,
        "Persisted expiry delta ({}s) too short; expected ~5s grant",
        delta_from_after
    );
    let expiry_not_extended = delta_from_before <= 6;
    assert!(
        expiry_not_extended,
        "Regression probe condition failed: expiry was extended beyond server lifetime"
    );
}

#[test]
fn test_cli_login_json_success_and_redaction() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");

    let server_response = http_resp(
        "200 OK",
        &[],
        r#"{"access_token":"canary-access-token-999","token_type":"Bearer","expires_in":3600,"refresh_token":"canary-refresh-token-888"}"#,
    );
    let (url, _rx) = canned_server(vec![server_response]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let out = Command::new(exe)
        .args([
            "login",
            "--username",
            "json-user@dosi.test",
            "--password",
            "canary-secret-pass-777",
            "--workspace-id",
            "ws-json",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute login --json command");

    assert_eq!(out.status.code(), Some(0));

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);

    // Verify stdout is pure parseable JSON
    let json: serde_json::Value = serde_json::from_str(&stdout_str).expect("Valid JSON on stdout");
    assert_eq!(json["success"], true);
    assert_eq!(json["account_id"], "json-user@dosi.test");
    assert_eq!(json["workspace_id"], "ws-json");
    assert_eq!(json["server_url"], url);
    assert_eq!(json["has_signing_key"], true);
    assert!(json["store_backend"].is_string());
    assert!(json["token_expires_in_secs"].as_i64().unwrap_or(0) > 0);
    assert!(json.get("error").is_none() || json["error"].is_null());

    // Verify zero secret canaries leaked in stdout or stderr
    assert!(!stdout_str.contains("canary-secret-pass-777"));
    assert!(!stderr_str.contains("canary-secret-pass-777"));
    assert!(!stdout_str.contains("canary-access-token-999"));
    assert!(!stderr_str.contains("canary-access-token-999"));
    assert!(!stdout_str.contains("canary-refresh-token-888"));
    assert!(!stderr_str.contains("canary-refresh-token-888"));

    // Verify credentials actually saved to disk
    let store = FileCredentialStore::new(cred_path);
    let saved = store
        .load("json-user@dosi.test", Some("ws-json"))
        .unwrap()
        .expect("Credentials must be saved");
    assert_eq!(saved.account_id, "json-user@dosi.test");
    assert_eq!(saved.workspace_id.as_deref(), Some("ws-json"));
    assert_eq!(
        saved.access_token.as_deref(),
        Some("canary-access-token-999")
    );
    assert_eq!(
        saved.refresh_token.as_deref(),
        Some("canary-refresh-token-888")
    );
    assert!(saved.device_signing_key.is_some());
}

#[test]
fn test_cli_login_json_auth_rejection_and_redaction() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");

    let server_response = http_resp(
        "401 Unauthorized",
        &[],
        r#"{"error":"invalid_grant","error_description":"Bad credentials"}"#,
    );
    let (url, _rx) = canned_server(vec![server_response]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let out = Command::new(exe)
        .args([
            "login",
            "--username",
            "rejected-json@dosi.test",
            "--password",
            "canary-rejected-pass-555",
            "--workspace-id",
            "ws-rejected-json",
            "--server",
            &url,
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute login --json command");

    assert_eq!(out.status.code(), Some(1));

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);

    // Verify stdout is pure parseable JSON with failure status
    let json: serde_json::Value = serde_json::from_str(&stdout_str).expect("Valid JSON on stdout");
    assert_eq!(json["success"], false);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("Authentication failed"),
        "Expected error message indicating auth failure, got: {:?}",
        json["error"]
    );

    // Verify zero secret canaries leaked in stdout or stderr
    assert!(!stdout_str.contains("canary-rejected-pass-555"));
    assert!(!stderr_str.contains("canary-rejected-pass-555"));

    // Verify store load returns None (zero credentials persisted)
    let store = FileCredentialStore::new(cred_path);
    assert_eq!(
        store
            .load("rejected-json@dosi.test", Some("ws-rejected-json"))
            .unwrap(),
        None
    );
}

#[test]
fn test_cli_login_json_credential_store_failure() {
    let dir = tempdir().unwrap();
    // Use an existing directory path as the credential file path to force store.save error
    let cred_dir_path = dir.path().to_path_buf();

    let server_response = token_ok();
    let (url, _rx) = canned_server(vec![server_response]);

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let out = Command::new(exe)
        .args([
            "login",
            "--username",
            "store-fail@dosi.test",
            "--password",
            "canary-store-fail-pass-444",
            "--workspace-id",
            "ws-store-fail",
            "--server",
            &url,
            "--credentials-file",
            cred_dir_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute login --json command");

    assert_eq!(out.status.code(), Some(1));

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);

    let json: serde_json::Value = serde_json::from_str(&stdout_str).expect("Valid JSON on stdout");
    assert_eq!(json["success"], false);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("Failed to securely store credentials"),
        "Expected store save error, got: {:?}",
        json["error"]
    );

    assert!(!stdout_str.contains("canary-store-fail-pass-444"));
    assert!(!stderr_str.contains("canary-store-fail-pass-444"));
}

#[test]
fn test_cli_logout_json_scoped_success_and_idempotence() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");
    let store = FileCredentialStore::new(cred_path.clone());

    let mut creds = Credentials::new("logout-user@dosi.test", Some("pass123".to_string()));
    creds.workspace_id = Some("ws-scoped".to_string());
    creds.server_url = Some("https://api.dosi.app".to_string());
    creds.access_token = Some("valid-token".to_string());
    creds.device_signing_key = Some(vec![1, 2, 3]);
    store.save(&creds).unwrap();

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. Scoped logout with --json
    let out = Command::new(exe)
        .args([
            "logout",
            "--account-id",
            "logout-user@dosi.test",
            "--workspace-id",
            "ws-scoped",
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute logout --json command");

    assert_eq!(out.status.code(), Some(0));

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout_str).expect("Valid JSON on stdout");
    assert_eq!(json["success"], true);
    assert_eq!(json["account_id"], "logout-user@dosi.test");
    assert_eq!(json["workspace_id"], "ws-scoped");
    assert_eq!(json["cleared_all_workspaces"], false);
    assert!(json["store_backend"].is_string());
    assert!(json.get("error").is_none() || json["error"].is_null());

    // Assert credentials cleared
    assert_eq!(
        store
            .load("logout-user@dosi.test", Some("ws-scoped"))
            .unwrap(),
        None
    );

    // 2. Idempotent re-invocation: same logout --json succeeds cleanly
    let out_again = Command::new(exe)
        .args([
            "logout",
            "--account-id",
            "logout-user@dosi.test",
            "--workspace-id",
            "ws-scoped",
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute idempotent logout --json command");

    assert_eq!(out_again.status.code(), Some(0));
    let stdout_again = String::from_utf8_lossy(&out_again.stdout);
    let json_again: serde_json::Value =
        serde_json::from_str(&stdout_again).expect("Valid JSON on idempotent logout stdout");
    assert_eq!(json_again["success"], true);
}

#[test]
fn test_cli_logout_json_preserves_cross_workspace_credentials() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");
    let store = FileCredentialStore::new(cred_path.clone());

    // Save ws-alpha
    let mut cred_alpha = Credentials::new("multi-ws@dosi.test", Some("pass-alpha".to_string()));
    cred_alpha.workspace_id = Some("ws-alpha".to_string());
    cred_alpha.access_token = Some("token-alpha".to_string());
    store.save(&cred_alpha).unwrap();

    // Save ws-beta
    let mut cred_beta = Credentials::new("multi-ws@dosi.test", Some("pass-beta".to_string()));
    cred_beta.workspace_id = Some("ws-beta".to_string());
    cred_beta.access_token = Some("token-beta".to_string());
    store.save(&cred_beta).unwrap();

    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // Clear ws-alpha only
    let out = Command::new(exe)
        .args([
            "logout",
            "--account-id",
            "multi-ws@dosi.test",
            "--workspace-id",
            "ws-alpha",
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute logout --json");

    assert_eq!(out.status.code(), Some(0));

    // ws-alpha must be gone
    assert_eq!(
        store.load("multi-ws@dosi.test", Some("ws-alpha")).unwrap(),
        None
    );

    // ws-beta must remain intact
    let loaded_beta = store
        .load("multi-ws@dosi.test", Some("ws-beta"))
        .unwrap()
        .expect("ws-beta credentials must remain untouched");
    assert_eq!(loaded_beta.access_token.as_deref(), Some("token-beta"));
}

#[test]
fn test_cli_login_and_logout_json_errors_and_canary_redaction() {
    let dir = tempdir().unwrap();
    let cred_path = dir.path().join("credentials.json");
    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    let canary_pos = "CANARY_SECRET_POS_VAL_12345";
    let canary_flag_val = "CANARY_SECRET_FLAG_VAL_67890";

    // 1. logout --json missing required --account-id
    let out_missing_acc = Command::new(exe)
        .args([
            "logout",
            "--workspace-id",
            "ws-test",
            "--credentials-file",
            cred_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to execute logout --json");

    assert_eq!(out_missing_acc.status.code(), Some(1));
    let stdout_missing = String::from_utf8_lossy(&out_missing_acc.stdout);
    let json_missing: serde_json::Value =
        serde_json::from_str(&stdout_missing).expect("Valid JSON on stdout");
    assert_eq!(json_missing["success"], false);
    assert!(json_missing["error"]
        .as_str()
        .unwrap()
        .contains("Missing required option: --account-id"));

    // 2. logout --json with secret positional argument and secret-valued flag
    let out_logout_canaries = Command::new(exe)
        .args([
            "logout",
            "--account-id",
            "user@dosi.test",
            canary_pos,
            &format!("--bogus-option={}", canary_flag_val),
            "--json",
        ])
        .output()
        .expect("Failed to execute logout --json");

    assert_eq!(out_logout_canaries.status.code(), Some(1));
    let stdout_logout_canary = String::from_utf8_lossy(&out_logout_canaries.stdout);
    let stderr_logout_canary = String::from_utf8_lossy(&out_logout_canaries.stderr);
    let json_logout_canary: serde_json::Value =
        serde_json::from_str(&stdout_logout_canary).expect("Valid JSON on stdout");
    assert_eq!(json_logout_canary["success"], false);

    assert!(!stdout_logout_canary.contains(canary_pos));
    assert!(!stderr_logout_canary.contains(canary_pos));
    assert!(!stdout_logout_canary.contains(canary_flag_val));
    assert!(!stderr_logout_canary.contains(canary_flag_val));

    // 3. login --json missing required options
    let out_login_missing = Command::new(exe)
        .args(["login", "--json"])
        .output()
        .expect("Failed to execute login --json");

    assert_eq!(out_login_missing.status.code(), Some(1));
    let stdout_login_missing = String::from_utf8_lossy(&out_login_missing.stdout);
    let json_login_missing: serde_json::Value =
        serde_json::from_str(&stdout_login_missing).expect("Valid JSON on stdout");
    assert_eq!(json_login_missing["success"], false);
    assert!(json_login_missing["error"]
        .as_str()
        .unwrap()
        .contains("Missing required option"));

    // 4. login --json with secret positional argument and secret-valued flag
    let out_login_canaries = Command::new(exe)
        .args([
            "login",
            "--username",
            "user@dosi.test",
            canary_pos,
            &format!("--unrecognized-flag={}", canary_flag_val),
            "--json",
        ])
        .output()
        .expect("Failed to execute login --json");

    assert_eq!(out_login_canaries.status.code(), Some(1));
    let stdout_login_canary = String::from_utf8_lossy(&out_login_canaries.stdout);
    let stderr_login_canary = String::from_utf8_lossy(&out_login_canaries.stderr);
    let json_login_canary: serde_json::Value =
        serde_json::from_str(&stdout_login_canary).expect("Valid JSON on stdout");
    assert_eq!(json_login_canary["success"], false);

    assert!(!stdout_login_canary.contains(canary_pos));
    assert!(!stderr_login_canary.contains(canary_pos));
    assert!(!stdout_login_canary.contains(canary_flag_val));
    assert!(!stderr_login_canary.contains(canary_flag_val));
}

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
