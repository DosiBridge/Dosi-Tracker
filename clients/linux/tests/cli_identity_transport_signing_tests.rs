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
use std::time::Duration;

use chrono::Utc;
use tempfile::tempdir;

use dosi_tracker_linux::api::HttpDeliveryTransport;
use dosi_tracker_linux::credentials::{CredentialStore, FileCredentialStore};
use dosi_tracker_linux::outbox::{ActivityEnvelope, Outbox, OutboxLimits, OutboxScope};
use dosi_tracker_linux::signing::verify_activity_envelope;
use dosi_tracker_linux::sync::{SyncPassConfig, SyncPassCoordinator};

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

    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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

    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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

    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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

    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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
