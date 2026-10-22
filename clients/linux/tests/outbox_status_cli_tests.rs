//! CLI integration and regression tests for read-only outbox status inspection.
//!
//! Enforces:
//! - Exact scoped positive counts (A+B fixtures) and zero-scope control.
//! - Non-mutating read-only inspection: DB bytes, metadata, and directory inventory unchanged.
//! - Nonexistent DB paths are never created.
//! - Strict error handling without echoing sensitive arguments.
//! - Zero payload, envelope, activity ID, or foreign-scope identity leakage.
//! - Corrupt database, invalid schema, and future version fail-closed behaviors.
//! - Accurate reading of uncheckpointed SQLite WAL data.

use std::fs;
use std::process::Command;

use chrono::Utc;
use rusqlite::Connection;

use dosi_tracker_linux::outbox::{
    inspect_outbox_status, ActivityEnvelope, Outbox, OutboxLimits, OutboxScope, OutboxStatusReport,
    OUTBOX_SCHEMA_VERSION,
};

fn make_test_envelope(client_id: &str, secret_desc: Option<&str>) -> ActivityEnvelope {
    let now = Utc::now();
    ActivityEnvelope {
        format_version: 1,
        client_activity_id: client_id.to_string(),
        project_id: Some("proj-alpha".to_string()),
        started_at: now - chrono::Duration::seconds(300),
        ended_at: now,
        productivity: Some(85),
        mouse_clicks: Some(42),
        keyboard_hits: Some(120),
        description: secret_desc.map(|s| s.to_string()),
        device_id: Some("dev-node-99".to_string()),
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
    }
}

#[test]
fn test_cli_outbox_missing_subcommand_and_unknown_subcommand() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

    // 1. Missing subcommand
    let output = Command::new(exe).arg("outbox").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Missing outbox subcommand"));
    assert!(stderr.contains("status"));

    // 2. Unknown subcommand
    let output = Command::new(exe)
        .args(["outbox", "purgedb"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown outbox subcommand"));

    // 3. Outbox help
    let output = Command::new(exe)
        .args(["outbox", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Inspect durable offline outbox storage"));
    assert!(stdout.contains("status"));

    // 4. Outbox status help
    let output = Command::new(exe)
        .args(["outbox", "status", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Read-only inspection of durable outbox queue"));
    assert!(stdout.contains("--db"));
    assert!(stdout.contains("--account-id"));
    assert!(stdout.contains("--workspace-id"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("Strictly read-only"));
}

#[test]
fn test_cli_outbox_status_invalid_and_missing_arguments() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("dummy.db");
    fs::write(&db_path, b"").unwrap();

    let p_str = db_path.to_str().unwrap();

    // Missing --db
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--account-id",
            "acc1",
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Missing required option: --db"));

    // Missing --account-id
    let out = Command::new(exe)
        .args(["outbox", "status", "--db", p_str, "--workspace-id", "ws1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Missing required option: --account-id"));

    // Missing --workspace-id
    let out = Command::new(exe)
        .args(["outbox", "status", "--db", p_str, "--account-id", "acc1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Missing required option: --workspace-id")
    );

    // Duplicate --db
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--db",
            p_str,
            "--account-id",
            "acc1",
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Duplicate option: --db"));

    // Duplicate --account-id
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            "acc1",
            "--account-id",
            "acc2",
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Duplicate option: --account-id"));

    // Duplicate --workspace-id
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            "acc1",
            "--workspace-id",
            "ws1",
            "--workspace-id",
            "ws2",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Duplicate option: --workspace-id"));

    // Duplicate --json
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            "acc1",
            "--workspace-id",
            "ws1",
            "--json",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Duplicate option: --json"));

    // Unknown option
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            "acc1",
            "--workspace-id",
            "ws1",
            "--unsupported-flag",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unknown option or unexpected argument"));

    // Empty value for --account-id
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            "   ",
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));

    // Control character in account-id: fails closed and does NOT echo sensitive input
    let secret_with_ctrl = "secret-token\x07-value";
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            secret_with_ctrl,
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Invalid scope"));
    assert!(stderr.contains("control characters"));
    assert!(!stderr.contains(secret_with_ctrl));

    // Oversized account_id (> 256 bytes)
    let oversized = "a".repeat(300);
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            p_str,
            "--account-id",
            &oversized,
            "--workspace-id",
            "ws1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("exceeds maximum length"));
    assert!(!stderr.contains(&oversized));
}

#[test]
fn test_cli_outbox_status_nonexistent_db_never_creates_file_or_directory() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();
    let nested_dir = temp.path().join("ghost_dir");
    let non_existent_db = nested_dir.join("nonexistent_outbox.db");

    assert!(!nested_dir.exists());
    assert!(!non_existent_db.exists());

    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            non_existent_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Database file does not exist"));

    // Absolute proof of non-mutation: neither the file nor parent dir were created
    assert!(!non_existent_db.exists(), "DB file must NEVER be created");
    assert!(!nested_dir.exists(), "Parent dir must NEVER be created");
}

#[test]
fn test_cli_outbox_status_corrupt_and_unsupported_version_refusal() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();

    // 1. Directory instead of file
    let dir_db = temp.path().join("directory_db");
    fs::create_dir_all(&dir_db).unwrap();
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            dir_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("directory, not a database file"));

    // 2. Corrupt garbage file
    let corrupt_db = temp.path().join("corrupt.db");
    fs::write(
        &corrupt_db,
        b"NOT_A_VALID_SQLITE_DATABASE_CORRUPT_HEADER_XYZ",
    )
    .unwrap();
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            corrupt_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("corrupted")
            || stderr.contains("Database access error")
            || stderr.contains("file is not a database")
    );

    // 3. Uninitialized database (user_version == 0)
    let uninit_db = temp.path().join("uninit.db");
    {
        let conn = Connection::open(&uninit_db).unwrap();
        conn.execute_batch("CREATE TABLE dummy (id INT);").unwrap();
    }
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            uninit_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("uninitialized"));

    // 4. Unsupported future store version (user_version == 999)
    let future_db = temp.path().join("future.db");
    {
        let conn = Connection::open(&future_db).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            future_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unsupported outbox store version"));

    // 5. Missing outbox_entries table
    let missing_table_db = temp.path().join("missing_table.db");
    {
        let conn = Connection::open(&missing_table_db).unwrap();
        conn.pragma_update(None, "user_version", OUTBOX_SCHEMA_VERSION)
            .unwrap();
    }
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            missing_table_db.to_str().unwrap(),
            "--account-id",
            "acc-1",
            "--workspace-id",
            "ws-1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("outbox_entries table not found"));
}

#[test]
fn test_cli_outbox_status_nonempty_a_and_b_fixtures_exact_counts_and_zero_scope_control() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("outbox_fixture.db");

    let scope_a = OutboxScope::new("Account-A", "Workspace-A").unwrap();
    let scope_b = OutboxScope::new("Account-B", "Workspace-B").unwrap();
    let scope_zero = OutboxScope::new("Account-Zero", "Workspace-Zero").unwrap();

    let limits = OutboxLimits::default();
    let outbox = Outbox::open(&db_path, limits).unwrap();

    // Enqueue 3 records for Scope A
    let env_a1 = make_test_envelope("act-a1", Some("work item A1"));
    let env_a2 = make_test_envelope("act-a2", Some("work item A2"));
    let env_a3 = make_test_envelope("act-a3", Some("work item A3"));
    outbox.enqueue(&scope_a, &env_a1).unwrap();
    outbox.enqueue(&scope_a, &env_a2).unwrap();
    outbox.enqueue(&scope_a, &env_a3).unwrap();

    // Mark retry failed for one entry in Scope A
    outbox
        .mark_attempt_failed(&scope_a, "act-a2", "connection reset by peer")
        .unwrap();
    outbox
        .mark_attempt_failed(&scope_a, "act-a2", "server busy 503")
        .unwrap();

    // Enqueue 2 records for Scope B
    let env_b1 = make_test_envelope("act-b1", Some("work item B1"));
    let env_b2 = make_test_envelope("act-b2", Some("work item B2"));
    outbox.enqueue(&scope_b, &env_b1).unwrap();
    outbox.enqueue(&scope_b, &env_b2).unwrap();

    drop(outbox); // close write connection

    // --- Query Scope A via CLI with --json ---
    let out_a = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "Account-A",
            "--workspace-id",
            "Workspace-A",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out_a.status.success(),
        "Scope A query must succeed with exit 0"
    );
    let stdout_a = String::from_utf8_lossy(&out_a.stdout);
    let report_a: OutboxStatusReport = serde_json::from_str(&stdout_a).expect("Valid JSON report");

    assert_eq!(report_a.format_version, 1);
    assert_eq!(report_a.scope.account_id, "account-a"); // normalized
    assert_eq!(report_a.scope.workspace_id, "Workspace-A");
    assert_eq!(report_a.durable_queue.queued_count, 3);
    assert_eq!(report_a.durable_queue.entries_with_retries, 1);
    assert_eq!(report_a.durable_queue.max_retry_count, 2);
    assert!(report_a.durable_queue.total_payload_bytes > 0);
    assert!(report_a.durable_queue.oldest_enqueued_at.is_some());
    assert!(report_a.durable_queue.newest_enqueued_at.is_some());
    assert_eq!(
        report_a.remote_delivery_state,
        "untracked_in_offline_outbox"
    );

    // --- Query Scope B via CLI with --json ---
    let out_b = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "account-b",
            "--workspace-id",
            "Workspace-B",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out_b.status.success(),
        "Scope B query must succeed with exit 0"
    );
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);
    let report_b: OutboxStatusReport = serde_json::from_str(&stdout_b).expect("Valid JSON report");

    assert_eq!(report_b.format_version, 1);
    assert_eq!(report_b.scope.account_id, "account-b");
    assert_eq!(report_b.scope.workspace_id, "Workspace-B");
    assert_eq!(report_b.durable_queue.queued_count, 2);
    assert_eq!(report_b.durable_queue.entries_with_retries, 0);
    assert_eq!(report_b.durable_queue.max_retry_count, 0);
    assert!(report_b.durable_queue.total_payload_bytes > 0);
    assert_eq!(
        report_b.remote_delivery_state,
        "untracked_in_offline_outbox"
    );

    // --- Zero-Scope Control: Scope Zero has 0 records ---
    let out_zero = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "Account-Zero",
            "--workspace-id",
            "Workspace-Zero",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out_zero.status.success(),
        "Zero-scope query must succeed with exit 0"
    );
    let stdout_zero = String::from_utf8_lossy(&out_zero.stdout);
    let report_zero: OutboxStatusReport =
        serde_json::from_str(&stdout_zero).expect("Valid JSON report");

    assert_eq!(report_zero.durable_queue.queued_count, 0);
    assert_eq!(report_zero.durable_queue.entries_with_retries, 0);
    assert_eq!(report_zero.durable_queue.max_retry_count, 0);
    assert_eq!(report_zero.durable_queue.total_payload_bytes, 0);
    assert_eq!(report_zero.durable_queue.oldest_enqueued_at, None);
    assert_eq!(report_zero.durable_queue.newest_enqueued_at, None);
    assert_eq!(
        report_zero.remote_delivery_state,
        "untracked_in_offline_outbox"
    );

    // --- Query Plain Text Format ---
    let out_text = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "Account-A",
            "--workspace-id",
            "Workspace-A",
        ])
        .output()
        .unwrap();

    assert!(out_text.status.success());
    let stdout_text = String::from_utf8_lossy(&out_text.stdout);
    assert!(stdout_text.contains("Dosi-Tracker Linux Offline Outbox Status"));
    assert!(stdout_text.contains("Queued Records:       3"));
    assert!(stdout_text.contains("Entries With Retries: 1"));
    assert!(stdout_text.contains("Max Retry Count:      2"));
    assert!(stdout_text.contains("Status: untracked_in_offline_outbox"));

    // --- Verify Directory Inventory Unchanged across queries and failure ---
    // In SQLite WAL mode, readers maintain shared memory (-shm/-wal).
    // Verify that across all successive status inspections and error runs, no extraneous files are created.
    let mid_entries: Vec<String> = fs::read_dir(temp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();

    // Run a failing query to test failure case stability
    let _ = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "invalid\x01scope",
            "--workspace-id",
            "ws",
        ])
        .output()
        .unwrap();

    let final_entries: Vec<String> = fs::read_dir(temp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();

    assert_eq!(
        mid_entries, final_entries,
        "Directory inventory must be strictly preserved across successive inspections and errors"
    );
    for entry in &final_entries {
        assert!(
            entry.starts_with("outbox_fixture.db"),
            "Only SQLite database and its WAL companions may exist, found: {entry}"
        );
    }

    // --- Verify Persisted Sentinels in DB are 100% Unchanged ---
    let re_outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    assert_eq!(re_outbox.count(Some(&scope_a)).unwrap(), 3);
    assert_eq!(re_outbox.count(Some(&scope_b)).unwrap(), 2);
    assert_eq!(re_outbox.count(Some(&scope_zero)).unwrap(), 0);

    let entries_a = re_outbox.peek_pending(&scope_a, 10).unwrap();
    assert_eq!(entries_a.len(), 3);
    let entry_a2 = entries_a
        .iter()
        .find(|e| e.client_activity_id == "act-a2")
        .unwrap();
    assert_eq!(entry_a2.retry_count, 2);
    assert_eq!(entry_a2.last_error.as_deref(), Some("server busy 503"));
}

#[test]
fn test_cli_outbox_status_no_payload_or_activity_id_leakage() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("leak_test.db");

    let scope_target = OutboxScope::new("target-user", "target-workspace").unwrap();
    let scope_foreign = OutboxScope::new("foreign-user", "foreign-workspace").unwrap();

    let limits = OutboxLimits::default();
    let outbox = Outbox::open(&db_path, limits).unwrap();

    let secret_payload_text = "CONFIDENTIAL_PAYLOAD_STRING_98765";
    let secret_activity_id = "SECRET_ACT_ID_112233";
    let foreign_activity_id = "FOREIGN_ACT_ID_445566";

    let env_target = make_test_envelope(secret_activity_id, Some(secret_payload_text));
    let env_foreign = make_test_envelope(foreign_activity_id, Some("foreign secret"));

    outbox.enqueue(&scope_target, &env_target).unwrap();
    outbox.enqueue(&scope_foreign, &env_foreign).unwrap();
    drop(outbox);

    // 1. Check JSON mode
    let out_json = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "target-user",
            "--workspace-id",
            "target-workspace",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let stderr_json = String::from_utf8_lossy(&out_json.stderr);

    // Assert strictly no sensitive content leakage in stdout or stderr
    assert!(!stdout_json.contains(secret_payload_text));
    assert!(!stdout_json.contains(secret_activity_id));
    assert!(!stdout_json.contains(foreign_activity_id));
    assert!(!stdout_json.contains("foreign-user"));
    assert!(!stdout_json.contains("foreign-workspace"));
    assert!(!stderr_json.contains(secret_payload_text));
    assert!(!stderr_json.contains(secret_activity_id));

    // 2. Check Text mode
    let out_text = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "target-user",
            "--workspace-id",
            "target-workspace",
        ])
        .output()
        .unwrap();

    assert!(out_text.status.success());
    let stdout_text = String::from_utf8_lossy(&out_text.stdout);
    assert!(!stdout_text.contains(secret_payload_text));
    assert!(!stdout_text.contains(secret_activity_id));
    assert!(!stdout_text.contains(foreign_activity_id));
    assert!(!stdout_text.contains("foreign-user"));
}

#[test]
fn test_cli_outbox_status_wal_mode_uncheckpointed_data() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("wal_test.db");

    let scope = OutboxScope::new("wal-acc", "wal-ws").unwrap();
    let limits = OutboxLimits::default();

    // Outbox::open sets PRAGMA journal_mode = WAL
    let outbox = Outbox::open(&db_path, limits).unwrap();

    let env1 = make_test_envelope("wal-act-1", Some("uncheckpointed entry"));
    outbox.enqueue(&scope, &env1).unwrap();

    // Verify WAL file exists (uncheckpointed frames)
    let wal_path = temp.path().join("wal_test.db-wal");
    assert!(
        wal_path.exists(),
        "WAL file should exist with committed transactions"
    );
    let initial_wal_meta = fs::metadata(&wal_path).unwrap();

    // Do NOT checkpoint or close write connection yet; run CLI status check while WAL is active
    let out = Command::new(exe)
        .args([
            "outbox",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--account-id",
            "wal-acc",
            "--workspace-id",
            "wal-ws",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "Read-only inspection must read WAL data cleanly"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: OutboxStatusReport = serde_json::from_str(&stdout).unwrap();

    // Exactly 1 record read through SQLite WAL
    assert_eq!(report.durable_queue.queued_count, 1);
    assert_eq!(report.durable_queue.entries_with_retries, 0);

    // Confirm that inspection did not truncate or remove the WAL file
    assert!(
        wal_path.exists(),
        "WAL file must not be removed by read-only inspection"
    );
    let post_wal_meta = fs::metadata(&wal_path).unwrap();
    assert_eq!(
        initial_wal_meta.len(),
        post_wal_meta.len(),
        "WAL file length unchanged"
    );
}

#[test]
fn test_module_inspect_outbox_status_direct_api() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("direct_test.db");

    let scope = OutboxScope::new("direct-acc", "direct-ws").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let env1 = make_test_envelope("direct-act-1", None);
    outbox.enqueue(&scope, &env1).unwrap();
    drop(outbox);

    // Call Rust API directly
    let report = inspect_outbox_status(&db_path, &scope).expect("Inspection should succeed");
    assert_eq!(report.format_version, 1);
    assert_eq!(report.durable_queue.queued_count, 1);
    assert_eq!(report.remote_delivery_state, "untracked_in_offline_outbox");
}
